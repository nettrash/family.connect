//! A server standing in for the real one, for the tests of the code that
//! TALKS to it — the halves of a feature no pure test reaches: which
//! requests are made, in what order, with what in them, and what the
//! client does with each answer.
//!
//! It stands in at the one place every request goes through: `fetch`
//! itself. So what runs under test is the shipped path whole — api.rs's
//! URL, its headers, its reading of the status and the body — against
//! answers a test writes down, with no network and no server on this
//! machine. The old `fetch` goes back when the stand-in is dropped.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::webcodecs::testing::Stand;

/// One request the client made.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub method: String,
    /// The path under `/api/v1`, query included: `/families/mine/pack`.
    pub path: String,
    /// The `Content-Type` it declared, or nothing.
    pub content_type: String,
    /// The body, byte for byte — empty for a request with none.
    pub body: Vec<u8>,
}

impl Asked {
    /// `GET /families/mine/pack`, as a test reads it.
    pub fn line(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    /// The body as the JSON it is.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }
}

/// What the stand-in answers.
pub enum Answer {
    /// A status and a JSON body.
    Json(u16, serde_json::Value),
    /// A status and raw bytes of a type — a picture.
    Bytes(u16, Vec<u8>, &'static str),
    /// No answer at all: the request fails as a dropped network does.
    Nothing,
}

impl Answer {
    /// The protocol's error shape (docs/protocol.md, "Error shape").
    pub fn refusal(status: u16, code: &str) -> Answer {
        Answer::Json(
            status,
            serde_json::json!({"error": {"code": code, "message": "for developers"}}),
        )
    }
}

/// The stand-in, for as long as it is held.
pub struct FakeServer {
    asked: Rc<RefCell<Vec<Asked>>>,
    _route: Closure<dyn FnMut(String, String, String, js_sys::Uint8Array) -> JsValue>,
    _fetch: Stand,
}

impl FakeServer {
    /// Answer every request with what `route` says for it.
    pub fn answering(route: impl Fn(&Asked) -> Answer + 'static) -> FakeServer {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let log = asked.clone();
        let route =
            Closure::<dyn FnMut(String, String, String, js_sys::Uint8Array) -> JsValue>::new(
                move |method: String,
                      url: String,
                      content_type: String,
                      body: js_sys::Uint8Array| {
                    // gloo-net ends every URL it builds with the separator
                    // for a query it was given none of; that is not the
                    // client's request, and a test should not have to
                    // spell it.
                    let path = url
                        .split_once("/api/v1")
                        .map_or(url.as_str(), |(_, path)| path)
                        .trim_end_matches(['&', '?'])
                        .to_string();
                    let request = Asked {
                        method,
                        path,
                        content_type,
                        body: body.to_vec(),
                    };
                    let answer = route(&request);
                    log.borrow_mut().push(request);
                    let (status, kind, payload): (u16, &str, JsValue) = match answer {
                        Answer::Json(status, body) => (
                            status,
                            "application/json",
                            JsValue::from_str(&body.to_string()),
                        ),
                        Answer::Bytes(status, bytes, kind) => (
                            status,
                            kind,
                            js_sys::Uint8Array::from(bytes.as_slice()).into(),
                        ),
                        Answer::Nothing => return JsValue::NULL,
                    };
                    js_sys::Array::of3(&JsValue::from(status), &JsValue::from_str(kind), &payload)
                        .into()
                },
            );
        // `fetch(request)` as gloo-net calls it: the method, the URL and
        // the body are read off the Request, and the answer is a Response
        // like any other. A status that may carry no body is given none.
        let make = js_sys::Function::new_with_args(
            "route",
            "return function (input) { return (async () => {
                 const body = new Uint8Array(await input.clone().arrayBuffer());
                 const type = input.headers.get('Content-Type') || '';
                 const answer = route(input.method, input.url, type, body);
                 if (answer === null) { throw new TypeError('Failed to fetch'); }
                 const bare = answer[0] === 204 || answer[0] === 304;
                 return new Response(bare ? null : answer[2],
                     { status: answer[0], headers: { 'Content-Type': answer[1] } });
             })(); };",
        );
        let fetch = make
            .call1(&JsValue::NULL, route.as_ref())
            .expect("the stand-in is a function");
        FakeServer {
            asked,
            _route: route,
            _fetch: Stand::in_for("fetch", &fetch),
        }
    }

    /// Every request made so far whose path begins with one of `under`,
    /// in order. Named rather than "all of them" on purpose: every test of
    /// this page shares one `fetch`, and a task an EARLIER test left
    /// running — a history read, a board sync — may make its request while
    /// this one is waiting on its own.
    pub fn asked(&self, under: &[&str]) -> Vec<Asked> {
        self.asked
            .borrow()
            .iter()
            .filter(|asked| under.iter().any(|prefix| asked.path.starts_with(prefix)))
            .cloned()
            .collect()
    }

    /// The same, as lines: `["GET /families/mine/pack"]`.
    pub fn lines(&self, under: &[&str]) -> Vec<String> {
        self.asked(under).iter().map(Asked::line).collect()
    }
}
