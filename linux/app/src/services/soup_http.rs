//! fc-core's [`Http`] over libsoup 3 (shell-design §2.4; carry-forward "For the shell's libsoup
//! seams", HTTP — implemented exactly as specified there):
//!
//! - **One `soup::Session`** per use (the API's; link previews get their own, see
//!   [`SoupHttp::for_link_previews`]), its own `timeout` left at 0: fc-core races every request
//!   against its own deadline and also passes it in `HttpRequest::timeout`, which this seam
//!   enforces too (the send future is dropped when it wins). User agent
//!   `FamilyConnect-Linux/<version>`. System trust store, no pinning, no invalid certificates;
//!   plain http to any host allowed; libsoup's default redirects; no cookie jar.
//! - **IDN**: a non-ASCII host goes through `g_hostname_to_ascii` ([`ascii_url`]).
//! - **Headers** appended as given; the body via `soup_message_set_request_body_from_bytes(msg,
//!   content_type, bytes)` — the `Content-Type` the request carries is that argument and is NOT
//!   appended a second time ([`split_content_type`]).
//! - **Cancelling is dropping**: `send_and_read_future` is a `GioFuture` whose `GCancellable` is
//!   cancelled when the future is dropped — a deadline winning drops it.
//! - **Any status is `Ok(HttpResponse)`**, with every header and the whole body.
//! - **Errors** ([`classify`]): TLS/certificate → `Tls`; resolver, refused, unreachable, reset,
//!   closed → `Unreachable`; `G_IO_ERROR_TIMED_OUT` → `Timeout`; else `Other`. The text is this
//!   module's own words for the error's kind — never libsoup's message, which may quote a host or a
//!   path, and never a token.

use std::time::Duration;

use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, TransportError};
use futures_util::future::{Either, select};
use glib::translate::IntoGlib;
use soup::prelude::*;

/// What every request of this app names itself: `FamilyConnect-Linux/1.3.0`.
pub fn user_agent() -> String {
    format!("FamilyConnect-Linux/{}", env!("CARGO_PKG_VERSION"))
}

/// HTTP for fc-core (the `rt::Http` seam), over one libsoup session.
#[derive(Debug, Clone)]
pub struct SoupHttp {
    session: soup::Session,
    follow_redirects: bool,
}

impl SoupHttp {
    /// The API's transport: libsoup's defaults (redirects followed), no cookie jar, the app's user
    /// agent, no session timeout (fc-core keeps its own deadlines).
    pub fn new() -> SoupHttp {
        SoupHttp {
            session: new_session(),
            follow_redirects: true,
        }
    }

    /// The link previews' own transport (carry-forward, "Animations"): its own session, NO
    /// redirects (`LinkPreviews` follows https redirects itself), no cookies, no credentials —
    /// nothing the API session has ever learnt can reach a stranger's host.
    pub fn for_link_previews() -> SoupHttp {
        SoupHttp {
            session: new_session(),
            follow_redirects: false,
        }
    }

    /// The session, for the WebSocket connector that shares it (`SoupSockets`).
    pub fn session(&self) -> &soup::Session {
        &self.session
    }
}

impl Default for SoupHttp {
    fn default() -> Self {
        SoupHttp::new()
    }
}

fn new_session() -> soup::Session {
    soup::Session::builder()
        .user_agent(user_agent())
        .timeout(0)
        .build()
}

impl Http for SoupHttp {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let session = self.session.clone();
        let follow_redirects = self.follow_redirects;
        Box::pin(async move {
            let message = build_message(&request, follow_redirects)?;
            let sending = session.send_and_read_future(&message, glib::Priority::DEFAULT);
            let read = match request.timeout {
                Some(limit) => match select(sending, Box::pin(deadline(limit))).await {
                    Either::Left((read, _)) => read,
                    // Dropping the send cancels it (GioFuture's cancellable).
                    Either::Right(((), _)) => return Err(TransportError::Timeout),
                },
                None => sending.await,
            };
            let body = read.map_err(|error| classify(&error))?;
            Ok(HttpResponse {
                status: status_of(&message),
                headers: response_headers(&message),
                body: body.to_vec(),
            })
        })
    }
}

async fn deadline(limit: Duration) {
    glib::timeout_future(limit).await;
}

/// The libsoup message for `request`: method, ASCII URL, headers, body.
fn build_message(
    request: &HttpRequest,
    follow_redirects: bool,
) -> Result<soup::Message, TransportError> {
    let url = ascii_url(&request.url)
        .ok_or_else(|| TransportError::Other("the request's address is not a URL".into()))?;
    let message = soup::Message::new(request.method.as_str(), &url)
        .map_err(|_| TransportError::Other("the request's address is not a URL".into()))?;
    if !follow_redirects {
        message.add_flags(soup::MessageFlags::NO_REDIRECT);
    }
    let (content_type, headers) = split_content_type(&request.headers);
    if let Some(sent) = message.request_headers() {
        for (name, value) in headers {
            sent.append(name, value);
        }
    }
    if let Some(body) = &request.body {
        let bytes = glib::Bytes::from(body.as_slice());
        message.set_request_body_from_bytes(content_type, Some(&bytes));
    }
    Ok(message)
}

/// The status as a number (`soup::Status` is an enum; its number is `into_glib`). A status beyond
/// `u16` (none exists) reads as 0.
fn status_of(message: &soup::Message) -> u16 {
    u16::try_from(message.status().into_glib()).unwrap_or(0)
}

/// Every response header, in libsoup's order, a repeated header once per value.
fn response_headers(message: &soup::Message) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(received) = message.response_headers() {
        received.foreach(|name, value| headers.push((name.to_string(), value.to_string())));
    }
    headers
}

/// The request's `Content-Type` (the first, compared without case), and every other header in
/// order. libsoup takes the content type with the body; appending it as well would send it twice.
pub fn split_content_type(headers: &[(String, String)]) -> (Option<&str>, Vec<(&str, &str)>) {
    let mut content_type = None;
    let mut rest = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            content_type.get_or_insert(value.as_str());
        } else {
            rest.push((name.as_str(), value.as_str()));
        }
    }
    (content_type, rest)
}

/// `url` with its host in ASCII (`g_hostname_to_ascii`: IDNA, `пример.рф` → `xn--e1afmkfd.xn--p1ai`);
/// the URL unchanged when the host is ASCII already; `None` when there is no `scheme://host` or the
/// host cannot be written in ASCII. IPv6 literals (`[…]`) and user info are kept as they are.
pub fn ascii_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (Some(userinfo), host_port),
        None => (None, authority),
    };
    if host_port.is_empty() {
        return None;
    }
    if host_port.is_ascii() {
        return Some(url.to_string());
    }
    // A non-ASCII host never carries brackets; its port, if any, follows the last colon.
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (host_port, None),
    };
    let ascii = glib::hostname_to_ascii(host)?;
    let mut out = format!("{scheme}://");
    if let Some(userinfo) = userinfo {
        out.push_str(userinfo);
        out.push('@');
    }
    out.push_str(&ascii);
    if let Some(port) = port {
        out.push(':');
        out.push_str(port);
    }
    out.push_str(tail);
    Some(out)
}

/// A libsoup/GIO error as fc-core's [`TransportError`] (see the module's comment).
pub fn classify(error: &glib::Error) -> TransportError {
    if let Some(tls) = error.kind::<gio::TlsError>() {
        let why = match tls {
            gio::TlsError::BadCertificate => "the certificate was refused",
            gio::TlsError::CertificateRequired => "the server asked for a certificate",
            gio::TlsError::NotTls => "the server does not speak TLS",
            gio::TlsError::Handshake => "the handshake failed",
            gio::TlsError::Eof => "the connection ended during the handshake",
            gio::TlsError::Unavailable => "TLS is not available",
            _ => "TLS failed",
        };
        return TransportError::Tls(why.to_string());
    }
    if error.kind::<gio::ResolverError>().is_some() {
        return TransportError::Unreachable("the host was not found".to_string());
    }
    if let Some(io) = error.kind::<gio::IOErrorEnum>() {
        let unreachable = |why: &str| TransportError::Unreachable(why.to_string());
        return match io {
            gio::IOErrorEnum::TimedOut => TransportError::Timeout,
            gio::IOErrorEnum::HostNotFound => unreachable("the host was not found"),
            gio::IOErrorEnum::ConnectionRefused => unreachable("the connection was refused"),
            gio::IOErrorEnum::HostUnreachable => unreachable("the host is unreachable"),
            gio::IOErrorEnum::NetworkUnreachable => unreachable("the network is unreachable"),
            gio::IOErrorEnum::BrokenPipe | gio::IOErrorEnum::Closed => {
                unreachable("the connection was closed")
            }
            gio::IOErrorEnum::NotConnected => unreachable("not connected"),
            gio::IOErrorEnum::ProxyFailed
            | gio::IOErrorEnum::ProxyAuthFailed
            | gio::IOErrorEnum::ProxyNeedAuth
            | gio::IOErrorEnum::ProxyNotAllowed => unreachable("the proxy refused"),
            gio::IOErrorEnum::Cancelled => TransportError::Other("cancelled".to_string()),
            other => TransportError::Other(format!("I/O error {other:?}")),
        };
    }
    if let Some(session) = error.kind::<soup::SessionError>() {
        return TransportError::Other(format!("HTTP session error {session:?}"));
    }
    TransportError::Other(format!("{} error", error.domain().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::rt::Method;

    #[test]
    fn an_idn_host_is_written_in_ascii_and_everything_else_is_kept() {
        assert_eq!(
            ascii_url("https://пример.рф/api/v1/me?x=1").as_deref(),
            Some("https://xn--e1afmkfd.xn--p1ai/api/v1/me?x=1")
        );
        assert_eq!(
            ascii_url("http://bücher.example:8080/api/v1/ws").as_deref(),
            Some("http://xn--bcher-kva.example:8080/api/v1/ws")
        );
        assert_eq!(
            ascii_url("https://chat.example.com/api/v1/chats/7/messages?after_id=3").as_deref(),
            Some("https://chat.example.com/api/v1/chats/7/messages?after_id=3")
        );
        // A path or query in another script is the server's business, not the host's.
        assert_eq!(
            ascii_url("https://chat.example.com/файл?q=ж").as_deref(),
            Some("https://chat.example.com/файл?q=ж")
        );
        assert_eq!(
            ascii_url("http://[::1]:8080/api/v1/me").as_deref(),
            Some("http://[::1]:8080/api/v1/me")
        );
        assert_eq!(ascii_url("chat.example.com/api"), None);
        assert_eq!(ascii_url("https:///api"), None);
        assert_eq!(ascii_url("://host/"), None);
    }

    #[test]
    fn the_content_type_travels_with_the_body_and_is_not_appended_twice() {
        let request = HttpRequest::new(Method::Post, "https://h.example/api/v1/auth/login")
            .header("Authorization", "Bearer t")
            .body("application/json", b"{}".to_vec())
            .header("X-Families", "3");
        let (content_type, rest) = split_content_type(&request.headers);
        assert_eq!(content_type, Some("application/json"));
        assert_eq!(
            rest,
            vec![("Authorization", "Bearer t"), ("X-Families", "3")]
        );
        let both = vec![
            ("content-type".to_string(), "image/png".to_string()),
            ("Content-Type".to_string(), "text/plain".to_string()),
        ];
        assert_eq!(split_content_type(&both), (Some("image/png"), vec![]));
        assert_eq!(split_content_type(&[]), (None, vec![]));
    }

    #[test]
    fn errors_are_mapped_by_kind_and_say_no_more_than_their_kind() {
        let io = |kind| classify(&glib::Error::new(kind, "https://host/secret/path?token=x"));
        assert_eq!(io(gio::IOErrorEnum::TimedOut), TransportError::Timeout);
        for kind in [
            gio::IOErrorEnum::ConnectionRefused,
            gio::IOErrorEnum::HostUnreachable,
            gio::IOErrorEnum::NetworkUnreachable,
            gio::IOErrorEnum::BrokenPipe,
            gio::IOErrorEnum::Closed,
            gio::IOErrorEnum::NotConnected,
            gio::IOErrorEnum::HostNotFound,
            gio::IOErrorEnum::ProxyFailed,
        ] {
            assert!(
                matches!(io(kind), TransportError::Unreachable(_)),
                "{kind:?}"
            );
        }
        assert!(matches!(
            io(gio::IOErrorEnum::PermissionDenied),
            TransportError::Other(_)
        ));
        let resolver = classify(&glib::Error::new(gio::ResolverError::NotFound, "h"));
        assert_eq!(
            resolver,
            TransportError::Unreachable("the host was not found".into())
        );
        let tls = classify(&glib::Error::new(gio::TlsError::BadCertificate, "h"));
        assert_eq!(
            tls,
            TransportError::Tls("the certificate was refused".into())
        );
        let session = classify(&glib::Error::new(soup::SessionError::Parsing, "h"));
        assert!(matches!(session, TransportError::Other(_)));
        for error in [
            io(gio::IOErrorEnum::ConnectionRefused),
            io(gio::IOErrorEnum::PermissionDenied),
            tls,
            session,
        ] {
            let text = error.to_string();
            assert!(!text.contains("secret") && !text.contains("token") && !text.contains("://"));
        }
    }

    #[test]
    fn the_user_agent_names_the_port_and_its_version() {
        assert_eq!(user_agent(), "FamilyConnect-Linux/1.3.0");
    }
}
