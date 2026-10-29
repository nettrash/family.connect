//! The `fc-rich` URI scheme (shell-design §9.1; md's `md-asset:` origin): `fc-rich://rich/<path>` is
//! answered by the app itself from the shipped `web/rich` folder (`Paths::rich_dir`,
//! `/usr/share/family-connect/rich` once installed) — so no request the page makes ever reaches a
//! network — and anything else is an empty 404.
//!
//! - Registered ONCE per process on the one `webkit::WebContext` (registrations leak within a
//!   process, md facts §3), through the security manager as SECURE and CORS-ENABLED — never
//!   `local`, which breaks module scripts.
//! - md's containment: the path after the host is split on `/`, and an empty segment, `.`, `..`, a
//!   backslash, a `%` (no escape is ever decoded into a path) or a NUL refuses the request; the
//!   query and the fragment are not part of the path. Only `fc-rich://rich/…` (scheme and host in
//!   either case) is ever looked at — `RichPage::allows`.
//! - Every response says `Cache-Control: no-store`; the MIME type comes from the extension (the
//!   table md uses); `rich.html` also carries its own content-security policy as a header, the same
//!   one its `<meta>` states.

use std::path::{Path, PathBuf};

use fc_logic::rich_drawing::RichPage;

/// The page's own policy (its `<meta>`), repeated as a header.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:";

/// The page's file inside the folder.
pub const PAGE_FILE: &str = "rich.html";

/// What one request is answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u32,
    pub mime: &'static str,
    pub body: Vec<u8>,
    /// Whether the response carries [`CONTENT_SECURITY_POLICY`] (the page only).
    pub csp: bool,
}

impl Answer {
    fn not_found() -> Answer {
        Answer {
            status: 404,
            mime: "text/plain",
            body: Vec::new(),
            csp: false,
        }
    }
}

/// The relative path `uri` names inside the folder, contained — or `None`.
pub fn contained_path(uri: &str) -> Option<String> {
    if !RichPage::allows(Some(uri)) {
        return None;
    }
    let (scheme, rest) = uri.split_once(':')?;
    if !scheme.eq_ignore_ascii_case(RichPage::SCHEME) {
        return None;
    }
    let after = rest.strip_prefix("//")?;
    let (host, path) = after.split_once('/')?;
    if !host.eq_ignore_ascii_case(RichPage::HOST) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    if path.is_empty() {
        return None;
    }
    let contained = path.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && !segment.contains(['\\', '%', '\0'])
    });
    contained.then(|| path.to_string())
}

/// The MIME type of a file by its extension.
pub fn mime_for(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

/// The answer to `uri` from the folder `rich_dir`.
pub fn answer(rich_dir: &Path, uri: &str) -> Answer {
    let Some(relative) = contained_path(uri) else {
        return Answer::not_found();
    };
    match std::fs::read(rich_dir.join(&relative)) {
        Ok(body) => Answer {
            status: 200,
            mime: mime_for(&relative),
            body,
            csp: relative == PAGE_FILE,
        },
        Err(_) => Answer::not_found(),
    }
}

thread_local! {
    static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Registers the scheme on `context` — once per process — as secure and CORS-enabled.
pub fn register(context: &webkit::WebContext, rich_dir: PathBuf) {
    if REGISTERED.with(|done| done.replace(true)) {
        return;
    }
    context.register_uri_scheme(RichPage::SCHEME, move |request| serve(&rich_dir, request));
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(RichPage::SCHEME);
        security.register_uri_scheme_as_cors_enabled(RichPage::SCHEME);
    }
}

fn serve(rich_dir: &Path, request: &webkit::URISchemeRequest) {
    let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
    let answer = answer(rich_dir, &uri);
    if answer.status != 200 {
        // Never the address: a path is not for the log.
        crate::fc_info!("rich: a request was refused ({})", answer.status);
    }
    let bytes = glib::Bytes::from_owned(answer.body);
    let length = i64::try_from(bytes.len()).unwrap_or(-1);
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let response = webkit::URISchemeResponse::new(&stream, length);
    response.set_status(
        answer.status,
        Some(if answer.status == 200 {
            "OK"
        } else {
            "Not Found"
        }),
    );
    response.set_content_type(answer.mime);
    let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
    headers.append("Cache-Control", "no-store");
    if answer.csp {
        headers.append("Content-Security-Policy", CONTENT_SECURITY_POLICY);
    }
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RichDir(PathBuf);

    impl RichDir {
        fn new(tag: &str) -> RichDir {
            let dir =
                std::env::temp_dir().join(format!("fc-rich-scheme-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("fonts")).unwrap();
            std::fs::write(dir.join("rich.html"), b"<html>").unwrap();
            std::fs::write(dir.join("fc-rich.js"), b"var VERSION = '1';").unwrap();
            std::fs::write(dir.join("fonts/KaTeX_Main-Regular.woff2"), b"wOF2").unwrap();
            RichDir(dir)
        }
    }

    impl Drop for RichDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_policy_is_the_pages_own() {
        let page = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/rich/rich.html"),
        )
        .expect("linux/data/rich/rich.html");
        assert!(page.contains(CONTENT_SECURITY_POLICY), "{page}");
    }

    #[test]
    fn the_page_and_its_files_are_served_with_their_types() {
        let rich = RichDir::new("serve");
        let page = answer(&rich.0, "fc-rich://rich/rich.html");
        assert_eq!((page.status, page.mime, page.csp), (200, "text/html", true));
        assert_eq!(page.body, b"<html>");
        let script = answer(&rich.0, "FC-RICH://RICH/fc-rich.js?v=1#x");
        assert_eq!(
            (script.status, script.mime, script.csp),
            (200, "text/javascript", false)
        );
        let font = answer(&rich.0, "fc-rich://rich/fonts/KaTeX_Main-Regular.woff2");
        assert_eq!((font.status, font.mime), (200, "font/woff2"));
        assert_eq!(answer(&rich.0, "fc-rich://rich/missing.js").status, 404);
    }

    #[test]
    fn nothing_outside_the_folder_or_the_origin_is_served() {
        let rich = RichDir::new("contain");
        for refused in [
            "fc-rich://rich/../Cargo.toml",
            "fc-rich://rich/fonts/../../x",
            "fc-rich://rich/%2e%2e/x",
            "fc-rich://rich//rich.html",
            "fc-rich://rich/./rich.html",
            "fc-rich://rich/fonts\\x",
            "fc-rich://rich/",
            "fc-rich://rich",
            "fc-rich://other/rich.html",
            "fc-rich://x@rich/rich.html",
            "fc-rich://rich:80/rich.html",
            "https://rich/rich.html",
            "file:///etc/passwd",
            "data:text/html,hi",
        ] {
            let refused_answer = answer(&rich.0, refused);
            assert_eq!(
                (refused_answer.status, refused_answer.body.len()),
                (404, 0),
                "{refused}"
            );
        }
    }

    #[test]
    fn types_follow_the_extension() {
        assert_eq!(mime_for("a.CSS"), "text/css");
        assert_eq!(mime_for("viz.wasm"), "application/wasm");
        assert_eq!(mime_for("README"), "application/octet-stream");
    }
}
