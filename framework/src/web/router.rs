//! Method and path to a controller, by segments: no regex, so building a
//! router (which every request does, on its fresh instance) is pushing
//! entries into a `Vec`.

use super::request::percent_decode;
use super::{Error, Method, Request, Response, method_name};

/// A controller.
pub type Handler = fn(Request) -> Result<Response, Error>;

/// Runs before a router's controllers, for auth: an `Err` is the response.
pub type Hook = fn(&Request) -> Result<(), Error>;

/// Routes from method and path to controllers.
///
/// - A pattern is `/`-separated segments: literal, `:name` (one segment,
///   read with [`Request::param`]) or a last `*name` (the rest of the path,
///   possibly empty). A trailing `/` in a request is ignored.
/// - When several routes match, a literal segment wins over `:name`, which
///   wins over `*name`, from the left.
/// - A path that matches only with another method answers 405 with
///   `Allow`; no match is a 404; both with `{"error": …}`. A `GET` route
///   answers `HEAD` too, without the body.
/// - [`Router::before`] adds a hook that runs before every controller of
///   this router (mounted ones included), in the order added.
/// - [`Router::mount`] puts another router under a prefix: an app's area or
///   an extension's routes, with its own hooks after this router's.
///
/// A route added twice, or a malformed pattern, panics when the router is
/// built: the app's first test that builds it shows it.
#[derive(Default)]
pub struct Router {
    routes: Vec<Route>,
    before: Vec<Hook>,
}

struct Route {
    method: Method,
    pattern: Vec<Segment>,
    handler: Handler,
    /// Hooks of the routers this one was mounted from, outermost first.
    hooks: Vec<Hook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
    Rest(String),
}

impl Router {
    pub fn new() -> Self {
        Router::default()
    }

    pub fn get(self, path: &str, handler: Handler) -> Self {
        self.route(Method::Get, path, handler)
    }

    pub fn post(self, path: &str, handler: Handler) -> Self {
        self.route(Method::Post, path, handler)
    }

    pub fn put(self, path: &str, handler: Handler) -> Self {
        self.route(Method::Put, path, handler)
    }

    pub fn patch(self, path: &str, handler: Handler) -> Self {
        self.route(Method::Patch, path, handler)
    }

    pub fn delete(self, path: &str, handler: Handler) -> Self {
        self.route(Method::Delete, path, handler)
    }

    /// A route for any method.
    pub fn route(mut self, method: Method, path: &str, handler: Handler) -> Self {
        let pattern = parse(path);
        self.add(Route { method, pattern, handler, hooks: Vec::new() }, path);
        self
    }

    /// Runs `hook` before every controller of this router.
    pub fn before(mut self, hook: Hook) -> Self {
        self.before.push(hook);
        self
    }

    /// Puts `router`'s routes under `prefix` (`/api/admin`; `:name`
    /// segments allowed), its hooks running after this router's.
    pub fn mount(mut self, prefix: &str, router: Router) -> Self {
        let prefix = parse(prefix);
        assert!(!prefix.iter().any(|s| matches!(s, Segment::Rest(_))), "a mount prefix can't have `*`");
        for route in router.routes {
            let pattern: Vec<Segment> = prefix.iter().cloned().chain(route.pattern).collect();
            let hooks = router.before.iter().copied().chain(route.hooks).collect();
            let shown = show(&pattern);
            self.add(Route { method: route.method, pattern, handler: route.handler, hooks }, &shown);
        }
        self
    }

    /// Answers one request: the matching controller's response, or the
    /// error's.
    pub fn handle(&self, raw: islas_sdk::Request) -> Response {
        let method = raw.method().clone();
        let path = raw.path().to_string();
        match self.dispatch(raw) {
            Ok(response) => response,
            Err(e) => e.into_response(&method, &path),
        }
    }

    fn dispatch(&self, raw: islas_sdk::Request) -> Result<Response, Error> {
        let Some(segments) = split(raw.path()) else {
            return Err(Error::BadRequest("bad path encoding".to_string()));
        };
        let head = *raw.method() == Method::Head;
        let mut found = self.best(&segments, raw.method());
        let mut drop_body = false;
        if found.is_none() && head {
            found = self.best(&segments, &Method::Get);
            drop_body = found.is_some();
        }
        let Some((route, params)) = found else {
            return self.no_route(&segments);
        };
        let req = Request::new(raw, params);
        for hook in self.before.iter().chain(&route.hooks) {
            hook(&req)?;
        }
        let response = (route.handler)(req)?;
        Ok(if drop_body { response.without_body() } else { response })
    }

    /// The most specific route for `method` matching `segments`, with its
    /// parameters.
    fn best(&self, segments: &[String], method: &Method) -> Option<(&Route, Vec<(String, String)>)> {
        self.routes
            .iter()
            .filter(|route| route.method == *method)
            .filter_map(|route| route.matches(segments).map(|params| (route, params)))
            .max_by(|(a, _), (b, _)| a.rank().cmp(&b.rank()))
    }

    /// 405 with `Allow` when another method has the path, else 404.
    fn no_route(&self, segments: &[String]) -> Result<Response, Error> {
        let mut allowed: Vec<&str> = Vec::new();
        for route in self.routes.iter().filter(|route| route.matches(segments).is_some()) {
            let mut names = vec![method_name(&route.method)];
            if route.method == Method::Get {
                names.push("HEAD");
            }
            for name in names {
                if !allowed.contains(&name) {
                    allowed.push(name);
                }
            }
        }
        if allowed.is_empty() {
            return Err(Error::NotFound);
        }
        let allow = allowed.join(", ");
        Ok(Response::json(&serde_json::json!({ "error": format!("method not allowed; allowed: {allow}") }))
            .with_status(405)
            .with_header("allow", allow))
    }

    fn add(&mut self, route: Route, shown: &str) {
        let twice = self.routes.iter().any(|r| r.method == route.method && same_shape(&r.pattern, &route.pattern));
        assert!(!twice, "route {} {shown} is added twice", method_name(&route.method));
        self.routes.push(route);
    }
}

impl Route {
    /// The parameters, if `segments` match the pattern.
    fn matches(&self, segments: &[String]) -> Option<Vec<(String, String)>> {
        let mut params = Vec::new();
        for (i, segment) in self.pattern.iter().enumerate() {
            match segment {
                Segment::Rest(name) => {
                    params.push((name.clone(), segments.get(i..).unwrap_or_default().join("/")));
                    return Some(params);
                }
                Segment::Literal(literal) => {
                    if segments.get(i) != Some(literal) {
                        return None;
                    }
                }
                Segment::Param(name) => params.push((name.clone(), segments.get(i)?.clone())),
            }
        }
        (segments.len() == self.pattern.len()).then_some(params)
    }

    /// Literal over `:name` over `*name`, from the left.
    fn rank(&self) -> Vec<u8> {
        self.pattern
            .iter()
            .map(|segment| match segment {
                Segment::Literal(_) => 2,
                Segment::Param(_) => 1,
                Segment::Rest(_) => 0,
            })
            .collect()
    }
}

/// A request path's segments, decoded; `None` for a bad escape.
fn split(path: &str) -> Option<Vec<String>> {
    let path = path.strip_suffix('/').unwrap_or(path);
    path.split('/').skip(1).map(percent_decode).collect()
}

fn parse(path: &str) -> Vec<Segment> {
    assert!(path.starts_with('/'), "route `{path}` must start with `/`");
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    let raw: Vec<&str> = if trimmed.is_empty() { Vec::new() } else { trimmed[1..].split('/').collect() };
    let last = raw.len().saturating_sub(1);
    raw.iter()
        .enumerate()
        .map(|(i, s)| {
            if let Some(name) = s.strip_prefix(':') {
                assert!(!name.is_empty(), "route `{path}`: a `:` needs a name");
                Segment::Param(name.to_string())
            } else if let Some(name) = s.strip_prefix('*') {
                assert!(!name.is_empty() && i == last, "route `{path}`: `*name` must be named and last");
                Segment::Rest(name.to_string())
            } else {
                assert!(!s.is_empty(), "route `{path}` has an empty segment");
                Segment::Literal(s.to_string())
            }
        })
        .collect()
}

/// Whether two patterns match the same paths (parameter names aside).
fn same_shape(a: &[Segment], b: &[Segment]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Segment::Literal(x), Segment::Literal(y)) => x == y,
            (Segment::Param(_), Segment::Param(_)) | (Segment::Rest(_), Segment::Rest(_)) => true,
            _ => false,
        })
}

fn show(pattern: &[Segment]) -> String {
    let mut shown = String::new();
    for segment in pattern {
        shown.push('/');
        match segment {
            Segment::Literal(s) => shown.push_str(s),
            Segment::Param(s) => shown.push_str(&format!(":{s}")),
            Segment::Rest(s) => shown.push_str(&format!("*{s}")),
        }
    }
    if shown.is_empty() { "/".to_string() } else { shown }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(req: Request) -> Result<Response, Error> {
        let params: Vec<String> = ["id", "name", "rest"]
            .iter()
            .filter_map(|n| req.param::<String>(n).ok().map(|v| format!("{n}={v}")))
            .collect();
        Ok(Response::text(format!("{} {}", req.path(), params.join(" "))))
    }

    fn other(_req: Request) -> Result<Response, Error> {
        Ok(Response::text("other"))
    }

    fn deny(req: &Request) -> Result<(), Error> {
        if req.header("authorization").is_some() { Ok(()) } else { Err(Error::Unauthorized) }
    }

    fn mark(req: &Request) -> Result<(), Error> {
        if req.header("x-order") == Some("first") { Ok(()) } else { Err(Error::Forbidden) }
    }

    fn call(router: &Router, method: Method, path: &str) -> (u16, String) {
        call_with(router, islas_sdk::Request::new(method, path))
    }

    fn call_with(router: &Router, raw: islas_sdk::Request) -> (u16, String) {
        let res = router.handle(raw);
        (res.status_code(), String::from_utf8(res.body().to_vec()).unwrap())
    }

    fn router() -> Router {
        Router::new()
            .get("/", other)
            .get("/api/notes", text)
            .post("/api/notes", other)
            .get("/api/notes/:id", text)
            .get("/api/notes/new", other)
            .delete("/api/notes/:id", text)
            .get("/files/*rest", text)
            .get("/files/:name", text)
    }

    #[test]
    fn params_and_catch_all() {
        let r = router();
        assert_eq!(call(&r, Method::Get, "/api/notes/42"), (200, "/api/notes/42 id=42".into()));
        assert_eq!(call(&r, Method::Get, "/api/notes/a%20b"), (200, "/api/notes/a%20b id=a b".into()));
        assert_eq!(call(&r, Method::Get, "/files/a/b/c"), (200, "/files/a/b/c rest=a/b/c".into()));
        assert_eq!(call(&r, Method::Get, "/files"), (200, "/files rest=".into()));
        assert_eq!(call(&r, Method::Get, "/api/notes/"), (200, "/api/notes/ ".into()), "a trailing / is ignored");
        assert_eq!(call(&r, Method::Get, "/").1, "other");
    }

    #[test]
    fn the_most_specific_route_wins() {
        let r = router();
        assert_eq!(call(&r, Method::Get, "/api/notes/new").1, "other", "literal over :id");
        assert_eq!(call(&r, Method::Delete, "/api/notes/new").1, "/api/notes/new id=new", "the method first");
        assert_eq!(call(&r, Method::Get, "/files/x").1, "/files/x name=x", ":name over *rest");
    }

    #[test]
    fn not_found_and_method_not_allowed() {
        let r = router();
        assert_eq!(call(&r, Method::Get, "/api/nothing"), (404, r#"{"error":"not found"}"#.into()));
        let res = r.handle(islas_sdk::Request::new(Method::Put, "/api/notes"));
        assert_eq!((res.status_code(), res.header("allow")), (405, Some("GET, HEAD, POST")));
        assert_eq!(
            String::from_utf8(res.body().to_vec()).unwrap(),
            r#"{"error":"method not allowed; allowed: GET, HEAD, POST"}"#
        );
        assert_eq!(call(&r, Method::Get, "/api/notes/%zz").0, 400);
    }

    #[test]
    fn head_is_get_without_the_body() {
        let res = router().handle(islas_sdk::Request::new(Method::Head, "/api/notes"));
        assert_eq!((res.status_code(), res.body()), (200, &b""[..]));
        assert_eq!(res.header("content-type"), Some("text/plain; charset=utf-8"));
    }

    #[test]
    fn mount_with_hooks_in_order() {
        let admin = Router::new().before(mark).get("/stats", other).get("/users/:id", text);
        let r = Router::new().before(deny).get("/open", other).mount("/api/admin", admin);
        let base = || islas_sdk::Request::new(Method::Get, "/api/admin/users/7");
        assert_eq!(call_with(&r, base()).0, 401, "this router's hook first");
        assert_eq!(call_with(&r, base().with_header("authorization", "x")).0, 403, "then the mounted one's");
        let both = base().with_header("authorization", "x").with_header("x-order", "first");
        assert_eq!(call_with(&r, both), (200, "/api/admin/users/7 id=7".into()));
        assert_eq!(call(&r, Method::Get, "/api/admin/nothing").0, 404, "no hook for a 404");
        assert_eq!(call(&r, Method::Get, "/open").0, 401, "hooks run for every route of the router");
    }

    #[test]
    #[should_panic(expected = "route GET /api/notes/:x is added twice")]
    fn a_route_added_twice_panics() {
        let _ = router().get("/api/notes/:x", other);
    }

    #[test]
    #[should_panic(expected = "must be named and last")]
    fn a_catch_all_in_the_middle_panics() {
        let _ = Router::new().get("/a/*rest/b", other);
    }
}
