//! Drawing the routes, as Rails does: `config/routes.rs` is a `draw` that
//! fills a [`Routes`] table, resources by their actions, the rest by verb.
//!
//! ```ignore
//! use islas::web::Action::*;
//! use islas::web::Routes;
//!
//! pub fn draw(r: &mut Routes) {
//!     r.scope("/api", |r| {
//!         r.resources::<notes::Notes>("notes", [Index, Create, Update, Destroy]);
//!         r.controller::<accounts::Accounts>(|c| {
//!             c.get("balances", Accounts::balances);
//!             c.post("transfer", Accounts::transfer);
//!         });
//!     });
//! }
//! ```
//!
//! Every controller is a type implementing [`Controller`], whose `before`
//! is what must hold before any of its actions (auth, access); one with
//! Rails' actions implements [`Resource`] too and is routed with
//! [`Routes::resources`], any other with [`Routes::controller`]. Hooks live
//! in the controller, never in the routes.

use std::mem;

use super::{Error, Handler, Method, Request, Response, Router};

/// The route table `draw` fills. Paths are relative to the scope they are
/// in (`"notes"` or `"/notes"`, both fine).
#[derive(Default)]
pub struct Routes {
    router: Router,
}

/// A controller: a type whose actions are associated functions
/// (`fn(Request) -> Result<Response, Error>`), and whose `before` runs
/// before each routed one: an `Err` is the response. The default allows.
/// A controller without a `before` is `impl Controller for Accounts {}`.
pub trait Controller {
    fn before(_req: &Request) -> Result<(), Error> {
        Ok(())
    }
}

/// A controller with Rails' actions, each with a default (404) so an
/// `impl` writes only what it has; which are routed is said in `draw`.
pub trait Resource: Controller {
    /// `GET /<path>`
    fn index(_req: Request) -> Result<Response, Error> {
        Err(Error::NotFound)
    }

    /// `GET /<path>/:id`
    fn show(_req: Request) -> Result<Response, Error> {
        Err(Error::NotFound)
    }

    /// `POST /<path>`
    fn create(_req: Request) -> Result<Response, Error> {
        Err(Error::NotFound)
    }

    /// `PATCH /<path>/:id`
    fn update(_req: Request) -> Result<Response, Error> {
        Err(Error::NotFound)
    }

    /// `DELETE /<path>/:id`
    fn destroy(_req: Request) -> Result<Response, Error> {
        Err(Error::NotFound)
    }
}

/// A resource's actions, for [`Routes::resources`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Index,
    Show,
    Create,
    Update,
    Destroy,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::Index, Action::Show, Action::Create, Action::Update, Action::Destroy];
}

impl Routes {
    /// Runs `draw` on an empty table and returns the router it filled.
    pub(crate) fn draw(draw: impl FnOnce(&mut Routes)) -> Router {
        let mut routes = Routes::default();
        draw(&mut routes);
        routes.router
    }

    pub fn get(&mut self, path: &str, handler: Handler) -> &mut Self {
        self.route(Method::Get, path, handler)
    }

    pub fn post(&mut self, path: &str, handler: Handler) -> &mut Self {
        self.route(Method::Post, path, handler)
    }

    pub fn put(&mut self, path: &str, handler: Handler) -> &mut Self {
        self.route(Method::Put, path, handler)
    }

    pub fn patch(&mut self, path: &str, handler: Handler) -> &mut Self {
        self.route(Method::Patch, path, handler)
    }

    pub fn delete(&mut self, path: &str, handler: Handler) -> &mut Self {
        self.route(Method::Delete, path, handler)
    }

    /// A route for any method.
    pub fn route(&mut self, method: Method, path: &str, handler: Handler) -> &mut Self {
        self.router = mem::take(&mut self.router).route(method, &absolute(path), handler);
        self
    }

    /// `T`'s custom actions, drawn by `draw` with `T::before` before each:
    /// `c.get("balances", Accounts::balances)`.
    pub fn controller<T: Controller>(&mut self, draw: impl FnOnce(&mut Routes)) -> &mut Self {
        let inner = Routes::draw(draw).before(T::before);
        self.router = mem::take(&mut self.router).mount("/", inner);
        self
    }

    /// Routes under a prefix: an area of the app, or an extension crate's
    /// `draw`.
    pub fn scope(&mut self, prefix: &str, draw: impl FnOnce(&mut Routes)) -> &mut Self {
        let inner = Routes::draw(draw);
        self.router = mem::take(&mut self.router).mount(&absolute(prefix), inner);
        self
    }

    /// `T`'s `actions` under `path`, with `T::before` before each.
    pub fn resources<T: Resource>(&mut self, path: &str, actions: impl IntoIterator<Item = Action>) -> &mut Self {
        let mut inner = Router::new().before(T::before);
        for action in actions {
            inner = match action {
                Action::Index => inner.get("/", T::index),
                Action::Show => inner.get("/:id", T::show),
                Action::Create => inner.post("/", T::create),
                Action::Update => inner.patch("/:id", T::update),
                Action::Destroy => inner.delete("/:id", T::destroy),
            };
        }
        self.router = mem::take(&mut self.router).mount(&absolute(path), inner);
        self
    }
}

fn absolute(path: &str) -> String {
    if path.starts_with('/') { path.to_string() } else { format!("/{path}") }
}

#[cfg(test)]
mod tests {
    use islas_sdk::Request as Raw;

    use super::*;

    struct Notes;

    impl Controller for Notes {
        fn before(req: &Request) -> Result<(), Error> {
            if req.header("authorization").is_none() { Err(Error::Unauthorized) } else { Ok(()) }
        }
    }

    impl Resource for Notes {
        fn index(_req: Request) -> Result<Response, Error> {
            Ok(Response::text("all"))
        }

        fn show(req: Request) -> Result<Response, Error> {
            Ok(Response::text(format!("note {}", req.param::<i64>("id")?)))
        }
    }

    struct Accounts;

    impl Controller for Accounts {
        fn before(req: &Request) -> Result<(), Error> {
            if req.header("authorization").is_none() { Err(Error::Forbidden) } else { Ok(()) }
        }
    }

    impl Accounts {
        fn balances(_req: Request) -> Result<Response, Error> {
            Ok(Response::text("balances"))
        }
    }

    fn open(_req: Request) -> Result<Response, Error> {
        Ok(Response::text("open"))
    }

    fn router() -> Router {
        Routes::draw(|r| {
            r.scope("/api", |r| {
                r.resources::<Notes>("notes", [Action::Index, Action::Show, Action::Create]);
                r.controller::<Accounts>(|c| {
                    c.get("balances", Accounts::balances);
                });
                r.get("open", open);
            });
            r.get("/", open);
        })
    }

    fn call(method: Method, path: &str, auth: bool) -> (u16, String) {
        let mut raw = Raw::new(method, path);
        if auth {
            raw = raw.with_header("authorization", "Bearer x");
        }
        let res = router().handle(raw);
        (res.status_code(), String::from_utf8_lossy(res.body()).into_owned())
    }

    #[test]
    fn resources_get_rails_actions_behind_before() {
        assert_eq!(call(Method::Get, "/api/notes", true), (200, "all".to_string()));
        assert_eq!(call(Method::Get, "/api/notes/7", true), (200, "note 7".to_string()));
        assert_eq!(call(Method::Get, "/api/notes", false).0, 401, "before runs first");
        assert_eq!(call(Method::Post, "/api/notes", true).0, 404, "routed but not implemented");
        assert_eq!(call(Method::Delete, "/api/notes/7", true).0, 405, "not routed");
        assert_eq!(call(Method::Get, "/api/balances", true), (200, "balances".to_string()));
        assert_eq!(call(Method::Get, "/api/balances", false).0, 403, "a controller's before, on a custom action");
        assert_eq!(call(Method::Get, "/api/open", false), (200, "open".to_string()), "no hook on a plain route");
        assert_eq!(call(Method::Get, "/", false).0, 200);
    }
}
