//! `Server.cs`: a server that answers by path, and the `ApiClient` the C# tests build over it.
//!
//! The match is the WHOLE path, query aside, and anchored for the reason the C# paid for once
//! already: a route that matched a substring answered `/chats/42/messages` with `/me`'s body —
//! `/messages` contains `/me` — and a fake that lies about which endpoint was asked can only teach
//! the wrong lesson. Like the C# handler (and unlike `fc_core::testing::FakeServer`) it keeps every
//! request BODY, in order, which is what the tests of "only what changed is sent" read. A request
//! no route answers never arrives (`Unreachable`, the C# handler's exception).

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::{ApiClient, MemoryTokenStore, ServerUrl};
use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, TransportError};
use fc_core::testing::{FakeClock, json_response};

type Route = Box<dyn Fn(&str) -> Option<(u16, Option<String>)>>;

/// A route that answers when it is ready (`OnAsync`): `None` lets the next route answer.
type AsyncRoute = Box<
    dyn Fn(&str) -> Option<LocalBoxFuture<'static, Result<(u16, Option<String>), TransportError>>>,
>;

#[derive(Default)]
struct State {
    routes: RefCell<Vec<Route>>,
    async_routes: RefCell<Vec<AsyncRoute>>,
    asked: RefCell<Vec<String>>,
    bodies: RefCell<Vec<String>>,
}

/// Clones share the routes and the record.
#[derive(Clone, Default)]
pub struct Server(Rc<State>);

/// `/api/v1/chats/42?x=1` → `/api/v1/chats/42`.
fn endpoint(path_and_query: &str) -> &str {
    path_and_query.split('?').next().unwrap_or(path_and_query)
}

/// The path and query of an absolute URL: everything from the first `/` after `://`.
fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.find('/').map_or("/", |at| &rest[at..])
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    /// `On(what, json)`: a `200` with this body for the endpoint `what` names (`"/me"` is
    /// `/api/v1/me`), every time it is asked.
    pub fn on(&self, what: &str, json: &str) -> &Self {
        self.on_status(what, 200, Some(json))
    }

    /// `On(what, json, status)`: this status, and this body or none.
    pub fn on_status(&self, what: &str, status: u16, json: Option<&str>) -> &Self {
        let wanted = format!("/api/v1{what}");
        let json = json.map(str::to_string);
        self.0.routes.borrow_mut().push(Box::new(move |path| {
            (endpoint(path) == wanted).then(|| (status, json.clone()))
        }));
        self
    }

    /// `Then(what, answers…)`: a different answer each time that endpoint is asked, the last one
    /// repeated once they run out.
    pub fn then(&self, what: &str, answers: &[(u16, Option<&str>)]) -> &Self {
        let wanted = format!("/api/v1{what}");
        let answers: Vec<(u16, Option<String>)> = answers
            .iter()
            .map(|(status, json)| (*status, json.map(str::to_string)))
            .collect();
        let asked = RefCell::new(0usize);
        self.0.routes.borrow_mut().push(Box::new(move |path| {
            if endpoint(path) != wanted {
                return None;
            }
            let mut count = asked.borrow_mut();
            let answer = answers[(*count).min(answers.len() - 1)].clone();
            *count += 1;
            Some(answer)
        }));
        self
    }

    /// `OnAsync(what, answer)`: the endpoint `what` names is answered by a future of its own — held
    /// until a test lets it go, or a transport failure (the C# handler throwing). Asked before the
    /// plain routes.
    pub fn on_async(
        &self,
        what: &str,
        answer: impl Fn() -> LocalBoxFuture<'static, Result<(u16, Option<String>), TransportError>>
        + 'static,
    ) -> &Self {
        let wanted = format!("/api/v1{what}");
        self.0.async_routes.borrow_mut().push(Box::new(move |path| {
            (endpoint(path) == wanted).then(&answer)
        }));
        self
    }

    /// The path and query of every request so far, oldest first (`Asked`).
    pub fn asked(&self) -> Vec<String> {
        self.0.asked.borrow().clone()
    }

    /// Every request body that carried one, in order (`Bodies`).
    pub fn bodies(&self) -> Vec<String> {
        self.0.bodies.borrow().clone()
    }
}

impl Http for Server {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let path = path_of(&request.url).to_string();
        self.0.asked.borrow_mut().push(path.clone());
        if let Some(body) = &request.body {
            self.0
                .bodies
                .borrow_mut()
                .push(String::from_utf8_lossy(body).into_owned());
        }
        let held = self
            .0
            .async_routes
            .borrow()
            .iter()
            .find_map(|route| route(&path));
        if let Some(held) = held {
            return Box::pin(async move {
                held.await
                    .map(|(status, json)| json_response(status, json.as_deref().unwrap_or(""), &[]))
            });
        }
        let answer = self
            .0
            .routes
            .borrow()
            .iter()
            .find_map(|route| route(&path))
            .map(|(status, json)| json_response(status, json.as_deref().unwrap_or(""), &[]))
            .ok_or_else(|| TransportError::Unreachable(format!("no route for {path}")));
        Box::pin(async move { answer })
    }
}

/// `new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new
/// MemoryTokenStore("t0ken"))`, its deadlines on `clock`.
pub fn api(server: &Server, clock: &FakeClock) -> Rc<ApiClient> {
    api_over(Rc::new(server.clone()), clock)
}

/// The same client over any transport.
pub fn api_over(http: Rc<dyn Http>, clock: &FakeClock) -> Rc<ApiClient> {
    Rc::new(ApiClient::new(
        http,
        ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(clock.clone()),
    ))
}

/// The C# `Pictures`/`Faces` handlers: one answer for ANY path — a status and a body — and the
/// path and query of every request kept.
#[derive(Clone)]
pub struct AnyPath {
    status: u16,
    body: Vec<u8>,
    asked: Rc<RefCell<Vec<String>>>,
}

impl AnyPath {
    pub fn new(status: u16, body: &str) -> Self {
        AnyPath {
            status,
            body: body.as_bytes().to_vec(),
            asked: Rc::default(),
        }
    }

    pub fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }
}

impl Http for AnyPath {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        self.asked
            .borrow_mut()
            .push(path_of(&request.url).to_string());
        let response = HttpResponse {
            status: self.status,
            headers: vec![("Content-Type".to_string(), "image/jpeg".to_string())],
            body: self.body.clone(),
        };
        Box::pin(async move { Ok(response) })
    }
}
