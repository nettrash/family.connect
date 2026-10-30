//! Helpers of the w2 logic-session package (notes): `Server.cs`'s `OnAsync` — a route whose answer
//! waits until the test lets it go ([`Gated`]).

use std::cell::RefCell;
use std::rc::Rc;

use futures_channel::oneshot;

use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, TransportError};

use super::server::Server;

/// A [`Server`] whose answer for ONE endpoint is held back until [`Gated::release`] — the C#'s
/// `OnAsync(what, () => gate.Task)`: the request is on the wire, and nothing has come back. The
/// first request to that endpoint waits; any later one is answered at once. Clones share the gate.
#[derive(Clone)]
pub struct Gated {
    inner: Server,
    endpoint: String,
    waiting: Rc<RefCell<Option<oneshot::Receiver<()>>>>,
    release: Rc<RefCell<Option<oneshot::Sender<()>>>>,
}

impl Gated {
    /// `what` names the endpoint as [`Server::on`] does (`"/attachments"` is
    /// `/api/v1/attachments`, query aside); the answer is whatever `inner` gives it.
    pub fn new(inner: Server, what: &str) -> Self {
        let (release, waiting) = oneshot::channel();
        Gated {
            inner,
            endpoint: format!("/api/v1{what}"),
            waiting: Rc::new(RefCell::new(Some(waiting))),
            release: Rc::new(RefCell::new(Some(release))),
        }
    }

    /// Let the held request through (`gate.SetResult(…)`).
    pub fn release(&self) {
        if let Some(release) = self.release.borrow_mut().take() {
            let _ = release.send(());
        }
    }

    /// What the server behind the gate was asked.
    pub fn server(&self) -> &Server {
        &self.inner
    }
}

/// The path of an absolute URL, query aside: `https://host/api/v1/x?y` → `/api/v1/x`.
fn endpoint_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    path.split('?').next().unwrap_or(path)
}

impl Http for Gated {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let held = if endpoint_of(&request.url) == self.endpoint {
            self.waiting.borrow_mut().take()
        } else {
            None
        };
        Box::pin(async move {
            if let Some(held) = held {
                let _ = held.await;
            }
            self.inner.send(request).await
        })
    }
}
