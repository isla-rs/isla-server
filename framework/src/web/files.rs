//! Files a route answers with: [`Response::file`](super::Response::file) and
//! `page` from the version's `static/` (preopened read-only at `/static`),
//! `data_file` and `download` from the app's `/data`. The response carries
//! `X-Islas-File: /static/<path>` (or `/data/<path>`) and no body, and the
//! host sends the file itself (`docs/abi.md` § HTTP requests): no copy
//! through the instance, no body cap. Natively (tests) the folders are the
//! project's `static/` (or what `testing::with_static` names) and a
//! temporary folder per thread (or `testing::with_data`), and
//! `testing::run_request` plays the host's part ([`as_the_host_sends`]).

use std::path::PathBuf;

use super::Error;

/// The header that names the file.
pub(crate) const HEADER: &str = "x-islas-file";

/// One of the app's two folders a response can name a file in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Folder {
    Static,
    Data,
}

impl Folder {
    /// Where the app sees it.
    fn guest(self) -> &'static str {
        match self {
            Folder::Static => "/static",
            Folder::Data => "/data",
        }
    }

    /// Where it is read and written from here.
    pub(crate) fn root(self) -> PathBuf {
        #[cfg(target_arch = "wasm32")]
        {
            PathBuf::from(self.guest())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            native::root(self)
        }
    }
}

/// The `X-Islas-File` value for `path` (relative, inside `folder`) if it
/// is a file there; a 404 for anything else, with the reason logged only
/// for a path that isn't one.
pub(crate) fn name(folder: Folder, path: &str) -> Result<String, Error> {
    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    let inside = !path.starts_with('/')
        && !path.contains('\0')
        && !segments.is_empty()
        && segments.iter().all(|segment| *segment != "." && *segment != "..");
    if !inside {
        eprintln!("islas: a file {path:?}: not a relative path inside {}", folder.guest());
        return Err(Error::NotFound);
    }
    let relative = segments.join("/");
    match std::fs::metadata(folder.root().join(&relative)) {
        Ok(metadata) if metadata.is_file() => Ok(format!("{}/{relative}", folder.guest())),
        _ => Err(Error::NotFound),
    }
}

/// `Content-Disposition: attachment` with `name` as the file's name: an
/// ASCII `filename` (quotes, `\`, `/`, controls and other characters as
/// `_`) and, when that isn't the name, the UTF-8 `filename*`.
pub(crate) fn attachment(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && !matches!(c, '"' | '\\' | '/') || c == ' ' { c } else { '_' })
        .collect();
    if ascii == name {
        return format!("attachment; filename=\"{ascii}\"");
    }
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(all(not(target_arch = "wasm32"), any(feature = "native", test)))]
pub(crate) use native::{as_the_host_sends, set_root};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::Folder;
    #[cfg(any(feature = "native", test))]
    use super::HEADER;

    /// Status, headers and body.
    #[cfg(any(feature = "native", test))]
    pub(crate) type Sent = (u16, Vec<(String, Vec<u8>)>, Vec<u8>);

    thread_local! {
        static STATIC: RefCell<PathBuf> = RefCell::new(PathBuf::from("static"));
        static DATA: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Where `folder` is in this thread (`testing::with_static`,
    /// `with_data`).
    #[cfg(any(feature = "native", test))]
    pub(crate) fn set_root(folder: Folder, dir: PathBuf) {
        match folder {
            Folder::Static => STATIC.with_borrow_mut(|root| *root = dir),
            Folder::Data => DATA.with_borrow_mut(|root| *root = Some(dir)),
        }
    }

    /// `/data` is a fresh temporary folder per thread until a test names one.
    pub(super) fn root(folder: Folder) -> PathBuf {
        match folder {
            Folder::Static => STATIC.with_borrow(|root| root.clone()),
            Folder::Data => DATA.with_borrow_mut(|root| {
                root.get_or_insert_with(|| {
                    let dir = std::env::temp_dir().join(format!("islas-data-{}", islas_sdk::random::token()));
                    std::fs::create_dir_all(&dir).expect("a temporary /data");
                    dir
                })
                .clone()
            }),
        }
    }

    /// What the host makes of a response that names a file, minus `ETag`
    /// and ranges: the file as the body (none for `HEAD`), the app's status
    /// and headers, `Content-Type` by extension and `Cache-Control:
    /// no-cache` unless set, `nosniff`, and `Content-Security-Policy:
    /// sandbox` for a `/data` file unless set; a 404 when it isn't there.
    #[cfg(any(feature = "native", test))]
    pub(crate) fn as_the_host_sends(
        status: u16,
        headers: &[(String, Vec<u8>)],
        body: &[u8],
        head: bool,
    ) -> Sent {
        let Some((_, value)) = headers.iter().find(|(name, _)| name == HEADER) else {
            return (status, headers.to_vec(), body.to_vec());
        };
        let value = String::from_utf8_lossy(value).into_owned();
        let found = [Folder::Static, Folder::Data].into_iter().find_map(|folder| {
            let relative = value.strip_prefix(folder.guest())?.strip_prefix('/')?;
            Some((folder, root(folder).join(relative)))
        });
        let not_found = || (404, vec![("content-type".to_string(), b"text/plain; charset=utf-8".to_vec())], b"Not Found".to_vec());
        let Some((folder, path)) = found else { return not_found() };
        let Ok(bytes) = std::fs::read(&path) else { return not_found() };
        let mut out: Vec<(String, Vec<u8>)> =
            headers.iter().filter(|(name, _)| name != HEADER && name != "content-length").cloned().collect();
        let mut default = |name: &str, value: &str| {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.to_string(), value.as_bytes().to_vec()));
            }
        };
        default("content-type", content_type(&value));
        default("cache-control", "no-cache");
        default("x-content-type-options", "nosniff");
        if folder == Folder::Data {
            default("content-security-policy", "sandbox");
        }
        (status, out, if head { Vec::new() } else { bytes })
    }

    /// The `Content-Type` for a file name, by its extension (the host's
    /// table).
    #[cfg(any(feature = "native", test))]
    pub(super) fn content_type(name: &str) -> &'static str {
        let name = name.rsplit('/').next().unwrap_or_default();
        let extension = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => extension,
            _ => "",
        };
        match extension.to_ascii_lowercase().as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "json" | "map" => "application/json",
            "webmanifest" => "application/manifest+json",
            "txt" => "text/plain; charset=utf-8",
            "md" => "text/markdown; charset=utf-8",
            "csv" => "text/csv; charset=utf-8",
            "xml" => "application/xml",
            "xhtml" => "application/xhtml+xml",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "avif" => "image/avif",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            "wasm" => "application/wasm",
            "pdf" => "application/pdf",
            "zip" => "application/zip",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            "wav" => "audio/wav",
            _ => "application/octet-stream",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_inside_the_folder() {
        let dir = std::env::temp_dir().join(format!("islas-files-{}", islas_sdk::random::token()));
        std::fs::create_dir_all(dir.join("admin")).unwrap();
        std::fs::write(dir.join("admin/index.html"), "<h1>admin</h1>").unwrap();
        std::fs::write(dir.join("secret.json"), "{}").unwrap();
        set_root(Folder::Static, dir.join("admin"));
        assert_eq!(name(Folder::Static, "index.html").unwrap(), "/static/index.html");
        assert_eq!(name(Folder::Static, "./index.html").ok(), None);
        for bad in ["../secret.json", "/etc/passwd", "", ".", "..", "nope.html", "a\0b", "admin/../../secret.json", "/"] {
            assert!(matches!(name(Folder::Static, bad), Err(Error::NotFound)), "{bad}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn data_is_a_folder_per_thread() {
        let root = Folder::Data.root();
        assert!(root.is_dir());
        assert_eq!(Folder::Data.root(), root, "the same one in the thread");
        let other = std::thread::spawn(|| Folder::Data.root()).join().unwrap();
        assert_ne!(other, root);
        std::fs::write(root.join("x.csv"), "a").unwrap();
        assert_eq!(name(Folder::Data, "x.csv").unwrap(), "/data/x.csv");
    }

    #[test]
    fn attachment_names() {
        assert_eq!(attachment("notes.csv"), "attachment; filename=\"notes.csv\"");
        assert_eq!(attachment("my notes.csv"), "attachment; filename=\"my notes.csv\"");
        assert_eq!(attachment("Ñotes \"q3\".csv"), "attachment; filename=\"_otes _q3_.csv\"; filename*=UTF-8''%C3%91otes%20%22q3%22.csv");
        assert_eq!(attachment("a/b\r\n.csv"), "attachment; filename=\"a_b__.csv\"; filename*=UTF-8''a%2Fb%0D%0A.csv");
    }

    #[test]
    fn types() {
        assert_eq!(native::content_type("/static/admin/index.html"), "text/html; charset=utf-8");
        assert_eq!(native::content_type("a.JS"), "text/javascript; charset=utf-8");
        assert_eq!(native::content_type("noext"), "application/octet-stream");
        assert_eq!(native::content_type("/data/.hidden"), "application/octet-stream");
    }
}
