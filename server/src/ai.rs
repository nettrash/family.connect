//! The assistant: Azure OpenAI chat completions (streamed) and image
//! generations.
//!
//! What leaves this server is deliberately narrow. A request carries the
//! configured system prompt and the last N messages of ONE member's OWN
//! assistant chat — never the family chat, never another member's thread
//! (docs/protocol.md, "The assistant"). That is the invariant which makes a
//! privacy-first family server able to talk to a hosted model at all, and it
//! is enforced here, at the only place that builds a request.
//!
//! Pictures are the same invariant drawn tighter (protocol.md, "Pictures").
//! A photograph rides on a turn only when the member attached it to the
//! question being answered, and it gets here already chosen and already
//! bounded: this file base64s bytes it is handed and never goes looking for
//! any. Generation is narrower still — the words after `/draw` and nothing
//! else, no prompt, no history, no language line.
//!
//! Since #56 the text model may also ASK for a picture itself, by calling
//! the one tool a draw-capable server declares ([`draw_picture_tool`]).
//! That does not move the decision about what leaves out of this server:
//! the question still goes to the text deployment, where it always went,
//! and what then goes to the images deployment is the tool's `prompt`
//! argument — a string this file read out of the stream, bounded, and
//! nothing else (protocol.md, "Drawing without being told to"). The model
//! decides WHETHER; the server still decides WHAT leaves and TO WHOM.
//!
//! WHICH DEPLOYMENT a request goes to is not decided here either. It arrives
//! as a [`ModelRoute`] built by `config.rs`, so "text, vision or images?" is
//! answered once, by the caller that knows what was asked, rather than three
//! times by three request builders.
//!
//! Streaming is server-sent events: `data: {json}` lines, terminated by
//! `data: [DONE]`. Parsed by hand rather than with an SSE crate — the format
//! is three rules, and a dependency that owns the parse would also own the
//! reconnect and retry behaviour we deliberately do not want.

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{AiImagesConfig, AuthScheme, ModelRoute};

/// Put the key on the request, in the header this route's provider reads.
///
/// Two schemes because two Azure surfaces disagree: classic Azure OpenAI
/// takes `api-key`, and the Foundry model surface takes an `Authorization:
/// Bearer` — with the SAME key. Which one is CONFIGURED, never sniffed from
/// the URL (see [`AuthScheme`]), so this function only obeys and there is
/// exactly one place in the file where a key meets a request.
fn with_key(request: reqwest::RequestBuilder, route: &ModelRoute) -> reqwest::RequestBuilder {
    match route.auth {
        AuthScheme::ApiKey => request.header("api-key", route.api_key.as_str()),
        AuthScheme::Bearer => request.bearer_auth(route.api_key.as_str()),
    }
}

/// One photograph, ready to travel.
///
/// Bytes and a media type, and nothing else — no id, no path, no name. What
/// reaches this struct has already been chosen by the member, filtered to
/// photos, preferred down to its preview and bounded in size by the caller
/// (`handlers_ai`); by the time it is here there is no decision left to make
/// and nothing for this file to look up. That is deliberate: the rule about
/// which pixels may leave lives in ONE place, and it is not this one.
#[derive(Debug, Clone)]
pub struct InlineImage {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl InlineImage {
    /// `data:image/jpeg;base64,…` — the form the chat-completions API takes
    /// for an image the caller holds rather than links to.
    ///
    /// A data URI rather than a URL because the alternative is worse in
    /// every direction: a family server is usually behind a home router, its
    /// attachment endpoint requires a session, and handing a provider a
    /// fetchable link to a family's photograph would be a second, permanent
    /// way in that outlives the request.
    fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime, BASE64.encode(&self.bytes))
    }
}

/// One turn of the conversation as the API wants it.
#[derive(Debug, Clone)]
pub struct ChatTurn {
    pub role: &'static str,
    pub content: String,
    /// Photographs riding on this turn. Empty on every turn but, at most,
    /// the last one — and empty always unless the family switched pictures
    /// on and the member attached one (protocol.md, "Pictures").
    pub images: Vec<InlineImage>,
}

impl ChatTurn {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant",
            content: content.into(),
            images: Vec::new(),
        }
    }

    /// The member's question with the photographs they attached to it.
    pub fn user_with_images(content: impl Into<String>, images: Vec<InlineImage>) -> Self {
        Self {
            role: "user",
            content: content.into(),
            images,
        }
    }

    /// This turn as the API wants it.
    ///
    /// **A turn with no images serialises exactly as it always did** — a
    /// plain string `content`, byte for byte the request a text-only server
    /// sent before pictures existed. That is not tidiness: the text
    /// deployment is a different model from the vision one, families are
    /// already talking to it, and "we changed the shape of every request to
    /// support a feature you have not enabled" is how a working assistant
    /// stops working.
    ///
    /// With images it becomes the multi-part form, text first. The text part
    /// is omitted when there is none, which is a photograph sent with no
    /// caption — the note in the system prompt is what tells the model what
    /// to do with that.
    fn to_json(&self) -> Value {
        if self.images.is_empty() {
            return json!({"role": self.role, "content": self.content});
        }
        let mut parts: Vec<Value> = Vec::with_capacity(self.images.len() + 1);
        if !self.content.trim().is_empty() {
            parts.push(json!({"type": "text", "text": self.content}));
        }
        for image in &self.images {
            parts.push(json!({
                "type": "image_url",
                "image_url": {"url": image.data_url()},
            }));
        }
        json!({"role": self.role, "content": parts})
    }
}

/// What a completed reply cost, for Family Statistics.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i32,
    #[serde(default)]
    pub completion_tokens: i32,
}

/// The name of the one tool a draw-capable server declares.
///
/// One tool, one name, known to the server: a call naming anything else is
/// refused rather than executed, because the server only ever offered this
/// one and a model inventing a second is not a request anybody made.
pub const DRAW_TOOL_NAME: &str = "draw_picture";

/// What the model is told the tool is for.
///
/// It carries the one fact the model cannot infer: the images model sees
/// NOTHING but the `prompt` — not this conversation, not any photograph —
/// so the prompt has to be complete in itself. Left unsaid, a model writes
/// "the cat from above, but in a hat" and the picture is of nothing.
const DRAW_TOOL_DESCRIPTION: &str = "Make a picture for the member. Call this when they ask for a picture, a drawing, an \
     image or an illustration, or when a picture is plainly the answer they want; answer in words \
     otherwise. The image model sees ONLY the prompt you pass — not this conversation and not any \
     photograph — so write a complete, self-contained description of the picture to make.";

/// The one tool, as the chat-completions API wants it declared.
///
/// Declared on a text request ONLY when the server has an images
/// deployment to honour a call with — the caller decides that, and a server
/// that cannot draw sends no `tools` key at all, so its requests stay byte
/// for byte what they were (protocol.md, "Drawing without being told to").
pub fn draw_picture_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": DRAW_TOOL_NAME,
            "description": DRAW_TOOL_DESCRIPTION,
            "parameters": {
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "A complete, self-contained description of the picture to make."
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        }
    })
}

/// A tool call the model made, accumulated from its stream deltas.
///
/// `arguments` is the RAW JSON text the model emitted, joined across
/// chunks, and it is parsed only once the stream has ended: a fragment of
/// JSON is not JSON, and the API sends the argument string a few characters
/// at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    /// The `prompt` of a `draw_picture` call, checked and bounded — the
    /// whole of what may then leave for the images deployment.
    ///
    /// Every way this can be wrong is an error rather than a guess: a call
    /// to a tool the server never declared, arguments that are not JSON, a
    /// `prompt` that is missing, not a string or blank, or one over
    /// `max_chars` — the message-body ceiling, because a prompt is the same
    /// kind of thing as the words after `/draw` and lives under the same
    /// bound. Nothing here is trimmed down or repaired: a model that
    /// decided to draw and then said nothing has failed the question, and
    /// the member is better served by `ai_error` and asking again than by a
    /// picture of whatever a repaired prompt happened to mean.
    pub fn draw_prompt(&self, max_chars: usize) -> Result<String> {
        if self.name != DRAW_TOOL_NAME {
            bail!(
                "the assistant called a tool this server did not offer: {:?}",
                self.name
            );
        }
        let arguments: Value = serde_json::from_str(&self.arguments)
            .context("the assistant's draw_picture arguments were not JSON")?;
        let Some(prompt) = arguments.get("prompt").and_then(Value::as_str) else {
            bail!("the assistant's draw_picture call carried no prompt string");
        };
        let prompt = prompt.trim();
        if prompt.is_empty() {
            bail!("the assistant's draw_picture call carried an empty prompt");
        }
        // Counted in characters and never sliced by a byte index: a prompt
        // is whatever the model wrote, in whatever alphabet.
        let chars = prompt.chars().count();
        if chars > max_chars {
            bail!(
                "the assistant's draw_picture prompt is {chars} characters, over the {max_chars} allowed"
            );
        }
        Ok(prompt.to_string())
    }
}

/// Everything a streamed reply came back with.
///
/// `text` is what streamed as words and `tool_call` is the tool the model
/// asked for instead, when it did. Both can be present — a model may say
/// "here you are" and then call the tool — and which of the two the reply
/// IS is not decided here: the caller owns that rule (protocol.md, "Drawing
/// without being told to").
#[derive(Debug, Clone, Default)]
pub struct Streamed {
    pub text: String,
    pub tool_call: Option<ToolCall>,
    pub usage: Usage,
}

/// The most tool calls one stream may accumulate. The server declares ONE
/// tool and honours ONE call; a stream indexing past this is a provider
/// misbehaving, and its later calls are dropped rather than allocated for.
const MAX_TOOL_CALLS: usize = 8;

/// The body of a chat-completions request, exactly as it is sent.
///
/// Its own function so the shape can be pinned without an HTTP stub: with
/// `tools` empty there is NO `tools` key — not an empty array, nothing — and
/// the body is byte for byte the one a text-only server has always sent.
/// That is the half of the tool-call feature that protects every family
/// whose server cannot draw: their requests do not change (protocol.md,
/// "Drawing without being told to").
fn request_body(
    route: &ModelRoute,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[Value],
) -> Value {
    let mut messages: Vec<Value> = Vec::with_capacity(turns.len() + 1);
    if !system_prompt.trim().is_empty() {
        messages.push(json!({"role": "system", "content": system_prompt}));
    }
    for turn in turns {
        messages.push(turn.to_json());
    }

    let mut body = json!({
        "messages": messages,
        "max_tokens": route.max_tokens,
        "stream": true,
        // Ask for the token counts in the final chunk; without this Azure
        // sends none when streaming and statistics would have nothing.
        "stream_options": {"include_usage": true},
        // The DEPLOYMENT name: Azure's v1 surface routes on it, and the
        // classic one ignores the field. The configured `model` is what gets
        // recorded with usage, not what is asked for.
        "model": route.model,
    });
    if !tools.is_empty() {
        // No `tool_choice`: the API's default is "auto", which is exactly the
        // rule — the model decides whether — and one field fewer is one field
        // fewer for a deployment to answer 400 to.
        body["tools"] = json!(tools);
    }
    body
}

/// Fold one server-sent event into the reply being accumulated.
///
/// Three things can be in an event and all three are read: the usage block
/// (Azure sends it alone, in a chunk with an empty `choices`), a content
/// delta (handed straight to `on_delta`), and tool-call deltas. The last
/// arrive as FRAGMENTS — an `index`, a `name` on the first chunk, and the
/// `arguments` string a few characters at a time — and are joined by index
/// until the stream ends. Only whole events are given to this function; the
/// line splitting is the caller's.
fn absorb_event<F>(
    event: &Value,
    reply: &mut Streamed,
    drafts: &mut Vec<ToolCall>,
    on_delta: &mut F,
) where
    F: FnMut(&str),
{
    if let Some(found) = event
        .get("usage")
        .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok())
    {
        reply.usage = found;
    }
    // Azure sends a first chunk with an empty `choices` array when it is
    // only reporting usage, so this is a `get`, not an index.
    let Some(delta) = event["choices"].get(0).map(|choice| &choice["delta"]) else {
        return;
    };
    if let Some(text) = delta["content"].as_str()
        && !text.is_empty()
    {
        reply.text.push_str(text);
        on_delta(text);
    }
    let Some(calls) = delta["tool_calls"].as_array() else {
        return;
    };
    for call in calls {
        // A missing index is the first call: some surfaces omit it when
        // there is only one.
        let Ok(index) = usize::try_from(call["index"].as_u64().unwrap_or(0)) else {
            continue;
        };
        if index >= MAX_TOOL_CALLS {
            continue;
        }
        while drafts.len() <= index {
            drafts.push(ToolCall::default());
        }
        let draft = &mut drafts[index];
        // The name is SET ONCE, never appended. Azure and OpenAI send it
        // whole on the first chunk and omit it after; a provider that
        // repeats it on every chunk — OpenAI-compatible proxies do — would
        // otherwise accumulate `draw_picturedraw_picture…`, which the
        // caller refuses as a tool it never declared, turning every
        // contextual draw on that provider into `ai_error`. A name that
        // arrives in fragments is not a shape any surface is known to
        // produce, and the arguments — which DO arrive in fragments — are
        // still joined below.
        if let Some(name) = call["function"]["name"].as_str()
            && draft.name.is_empty()
        {
            draft.name.push_str(name);
        }
        if let Some(arguments) = call["function"]["arguments"].as_str() {
            draft.arguments.push_str(arguments);
        }
    }
}

/// The stream is over: the FIRST tool call with anything in it is the one
/// the reply made. One tool was declared and one call is honoured; a model
/// emitting several has asked for one picture several times, and the
/// second and later are dropped rather than drawn.
fn finish(mut reply: Streamed, drafts: Vec<ToolCall>) -> Streamed {
    reply.tool_call = drafts
        .into_iter()
        .find(|draft| !draft.name.is_empty() || !draft.arguments.is_empty());
    reply
}

/// Stream a reply, handing each fragment to `on_delta` as it arrives.
///
/// `on_delta` is called on the caller's task, so it should do nothing slow —
/// fanning a frame out to the member's sockets is exactly the right amount
/// of work.
///
/// Returns the full text, the tool call the model made instead if it made
/// one, and what it cost. An error mid-stream still leaves whatever arrived
/// with the caller through `on_delta`; the partial answer is worth more than
/// nothing, and the caller reports it as such.
///
/// `tools` is what the model may call — [`draw_picture_tool`] on a server
/// that can draw, and EMPTY on one that cannot, which sends no `tools` key
/// at all. A tool call arrives as deltas like the words do and is only whole
/// once the stream is; it is handed back unparsed, for the caller to check.
pub async fn stream_reply<F>(
    client: &reqwest::Client,
    route: &ModelRoute,
    system_prompt: &str,
    turns: &[ChatTurn],
    tools: &[Value],
    mut on_delta: F,
) -> Result<Streamed>
where
    F: FnMut(&str),
{
    let body = request_body(route, system_prompt, turns, tools);

    let url = &route.url;
    let response = with_key(client.post(url), route)
        .json(&body)
        .send()
        .await
        .context("calling the assistant")?;

    let status = response.status();
    if !status.is_success() {
        // The URL goes in the message, and it is what makes a 404
        // diagnosable: "Resource not found" alone cannot tell you whether
        // the endpoint, the deployment or the api-version is wrong. None of
        // it is secret — the key is only ever a header.
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "assistant returned {status} for {}: {}",
            url,
            detail.chars().take(400).collect::<String>()
        );
    }

    let mut reply = Streamed::default();
    let mut drafts: Vec<ToolCall> = Vec::new();
    let mut buffer = String::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the assistant's stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Events are separated by a blank line, and a chunk boundary can
        // land anywhere — including mid-line — so only WHOLE lines are
        // taken and the remainder stays in the buffer.
        while let Some(newline) = buffer.find('\n') {
            let line = buffer[..newline].trim_end_matches('\r').to_string();
            buffer.drain(..=newline);

            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                return Ok(finish(reply, drafts));
            }
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                // A fragment we cannot parse is not worth failing a reply
                // over; the next one usually carries on fine.
                continue;
            };
            absorb_event(&event, &mut reply, &mut drafts, &mut on_delta);
        }
    }

    Ok(finish(reply, drafts))
}

/// A picture, as it came back.
#[derive(Debug, Clone)]
pub struct GeneratedImage {
    /// Sniffed from the bytes, never trusted from the response: the media
    /// type is what the attachment row will claim and what every client will
    /// render from, and this server's rule everywhere else is that the bytes
    /// decide (`handlers_attachment::matches_magic`).
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Ask the images deployment for one picture.
///
/// `prompt` is the whole of what leaves the server: the words after `/draw`,
/// as the member typed them. No system prompt, no thread, no transcript, no
/// language instruction — an image has no language to answer in, and there
/// is nothing here for a family's history to add (protocol.md, "Pictures").
///
/// **The request body varies by deployment, so every part of it that does is
/// configured rather than assumed, and omitted when left empty.** `size`,
/// `width`/`height` and `response_format` are the fields image models
/// disagree about, and an endpoint rejecting one it does not implement
/// answers 400 with nothing a family can act on. Two live examples of the
/// disagreement: the OpenAI images contract takes `size: "1024x1024"`, while
/// FLUX on Azure Foundry takes `width` and `height` as integers and refuses
/// `size` — so the omission rule is what lets one `[ai.images]` section
/// speak to either. Both response shapes are parsed whichever arrives, so
/// asking for neither `response_format` is the safe default.
pub async fn generate_image(
    client: &reqwest::Client,
    route: &ModelRoute,
    cfg: &AiImagesConfig,
    prompt: &str,
) -> Result<GeneratedImage> {
    let mut body = json!({
        "prompt": prompt,
        // One. A second picture is a second bill for something nobody asked
        // for, and the reply is one message with one attachment.
        "n": 1,
        "model": route.model,
    });
    if !cfg.size.trim().is_empty() {
        body["size"] = json!(cfg.size.trim());
    }
    // The other spelling of the same thing, for the deployments that take it
    // apart. Each half stands alone — a model that has a default for one and
    // not the other is a config that sets one and not the other.
    if cfg.width > 0 {
        body["width"] = json!(cfg.width);
    }
    if cfg.height > 0 {
        body["height"] = json!(cfg.height);
    }
    if !cfg.response_format.trim().is_empty() {
        body["response_format"] = json!(cfg.response_format.trim());
    }

    let response = with_key(client.post(&route.url), route)
        .json(&body)
        .send()
        .await
        .context("asking the assistant for a picture")?;

    let status = response.status();
    if !status.is_success() {
        // The URL goes in the message for the reason it does in
        // `stream_reply`: a bare 404 or 400 from an images endpoint cannot
        // say whether the endpoint, the deployment, the api-version or a
        // body field this deployment does not implement was the wrong one.
        // None of it is secret — the key is only ever a header.
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "image generation returned {status} for {}: {}",
            route.url,
            detail.chars().take(400).collect::<String>()
        );
    }

    let payload: Value = response
        .json()
        .await
        .context("reading the picture the assistant made")?;
    let first = payload["data"]
        .get(0)
        .ok_or_else(|| anyhow::anyhow!("image response carried no data[0]"))?;

    // Both shapes, because which one arrives depends on the deployment and
    // on whether `response_format` was sent at all. `b64_json` is the bytes
    // inline; `url` is a short-lived link on the provider's own storage,
    // which the SERVER fetches — never a client, and never the family.
    let bytes = if let Some(encoded) = first["b64_json"].as_str() {
        let bytes = BASE64
            .decode(encoded)
            .context("the picture was not valid base64")?;
        if bytes.len() > cfg.max_bytes {
            bail!(
                "generated picture is {} bytes, over the {} the server allows",
                bytes.len(),
                cfg.max_bytes
            );
        }
        bytes
    } else if let Some(url) = first["url"].as_str() {
        download(client, url, cfg.max_bytes).await?
    } else {
        bail!("image response carried neither b64_json nor url");
    };

    let Some(mime) = sniff_image(&bytes) else {
        // Refused rather than stored under a guessed type. An attachment
        // whose media type is wrong is a grey box on every client in the
        // family, and this server accepts exactly the types it can name
        // (`models::Attachment::ACCEPTED`).
        bail!("the picture is neither a PNG nor a JPEG");
    };
    Ok(GeneratedImage { mime, bytes })
}

/// Fetch a generated picture from the link the provider gave, bounded.
///
/// Chunk by chunk with a running total rather than `bytes()` whole, for the
/// same reason uploads are streamed: the ceiling has to bind before the
/// allocation, not after it. The key rides on NEITHER header here, whatever
/// the route's scheme — the link is pre-signed, and sending the key to
/// whatever host a response named would be handing it over on the provider's
/// say-so.
async fn download(client: &reqwest::Client, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .context("fetching the picture the assistant made")?;
    let status = response.status();
    if !status.is_success() {
        bail!("fetching the generated picture returned {status}");
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the generated picture")?;
        if bytes.len() + chunk.len() > max_bytes {
            bail!("generated picture is over the {max_bytes} bytes the server allows");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// What these bytes actually are, or `None`.
///
/// Two types, because those are the two an images endpoint returns and the
/// two every client in this family already draws. The check is the same
/// magic-number check an upload gets — the declared type is never the
/// evidence here either.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    // The URL tests below predate routes and still assert against the
    // config, which is where a URL is built; the request-shape tests after
    // them assert against what this file serialises.
    use crate::config::AiConfig;

    #[test]
    fn the_url_is_built_from_the_deployment_not_the_model() {
        let cfg = AiConfig {
            enabled: true,
            endpoint: "https://example.openai.azure.com/".to_string(),
            deployment: "my-deployment".to_string(),
            model: "gpt-oss-120b".to_string(),
            api_key: "secret".to_string(),
            api_version: "2024-10-21".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.completions_url(),
            "https://example.openai.azure.com/openai/deployments/my-deployment\
             /chat/completions?api-version=2024-10-21"
        );
    }

    /// Azure has more than one endpoint shape, and a wrong guess is a bare
    /// "404 Resource not found". A pasted target URI is used as given.
    #[test]
    fn an_endpoint_that_is_already_a_full_url_is_used_verbatim() {
        let base = AiConfig {
            enabled: true,
            api_key: "secret".to_string(),
            api_version: "2024-10-21".to_string(),
            ..Default::default()
        };

        // AI Foundry / serverless shape, pasted whole. The deployment is
        // NOT spliced in — the URL already says where to go.
        let foundry = AiConfig {
            endpoint: "https://my-resource.services.ai.azure.com/models/chat/completions"
                .to_string(),
            deployment: "ignored-here".to_string(),
            ..base.clone()
        };
        assert_eq!(
            foundry.completions_url(),
            "https://my-resource.services.ai.azure.com/models/chat/completions\
             ?api-version=2024-10-21"
        );

        // A pasted URL that already carries its own query keeps it, rather
        // than getting a second `?`.
        let with_query = AiConfig {
            endpoint: "https://x.example/models/chat/completions?api-version=2025-01-01"
                .to_string(),
            ..base
        };
        assert_eq!(
            with_query.completions_url(),
            "https://x.example/models/chat/completions?api-version=2025-01-01"
        );
    }

    /// Azure's v1 (OpenAI-compatible) surface, which is what an endpoint
    /// ending in `/openai/v1` is.
    ///
    /// The deployment goes in the BODY, not the path, and there is no
    /// `api-version` query. Splicing the classic `/openai/deployments/…`
    /// onto one of these gives a doubled `/openai` and a bare
    /// "404 Resource not found" — which is exactly what happened in
    /// production.
    #[test]
    fn the_v1_surface_puts_the_deployment_in_the_body_not_the_path() {
        let cfg = AiConfig {
            enabled: true,
            endpoint: "https://nettrash-openai.openai.azure.com/openai/v1".to_string(),
            deployment: "nettrash-gpt-oss-120b".to_string(),
            model: "nettrash-gpt-oss-120b".to_string(),
            api_key: "secret".to_string(),
            api_version: "2024-10-21".to_string(),
            ..Default::default()
        };

        assert_eq!(
            cfg.completions_url(),
            "https://nettrash-openai.openai.azure.com/openai/v1/chat/completions",
            "no deployments path, and no api-version query"
        );
        assert_eq!(
            cfg.request_model(),
            "nettrash-gpt-oss-120b",
            "the deployment is what routes the request"
        );
    }

    /// A trailing slash must not change the shape.
    #[test]
    fn a_trailing_slash_on_a_v1_endpoint_is_ignored() {
        let cfg = AiConfig {
            endpoint: "https://x.openai.azure.com/openai/v1/".to_string(),
            deployment: "d".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.completions_url(),
            "https://x.openai.azure.com/openai/v1/chat/completions"
        );
    }

    /// The shape a text-only request has always had, asserted because a
    /// second shape now exists. The text deployment is a DIFFERENT model
    /// from the vision one, families are already talking to it, and
    /// changing every request to support a feature nobody enabled is how a
    /// working assistant stops working.
    #[test]
    fn a_turn_without_images_is_a_plain_string_exactly_as_before() {
        let turn = ChatTurn::user("what is the weather");
        assert_eq!(
            turn.to_json(),
            json!({"role": "user", "content": "what is the weather"})
        );
        assert_eq!(
            ChatTurn::assistant("cold").to_json(),
            json!({"role": "assistant", "content": "cold"})
        );
    }

    /// With photographs it becomes the multi-part form, text FIRST, and the
    /// images inline as data URIs — never as links back to this server,
    /// which is behind a home router and would need a session anyway.
    #[test]
    fn a_turn_with_images_carries_them_inline_after_the_text() {
        let turn = ChatTurn::user_with_images(
            "what is this?",
            vec![InlineImage {
                mime: "image/jpeg".to_string(),
                bytes: vec![0xFF, 0xD8, 0xFF],
            }],
        );
        assert_eq!(
            turn.to_json(),
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,/9j/"}},
                ]
            })
        );
    }

    /// A photograph sent with no caption. The text part is omitted rather
    /// than sent empty — an empty string is a turn that says nothing, and
    /// what tells the model what to do with a wordless picture is the note
    /// in the system prompt.
    #[test]
    fn a_wordless_picture_carries_no_text_part() {
        let turn = ChatTurn::user_with_images(
            "   ",
            vec![InlineImage {
                mime: "image/png".to_string(),
                bytes: vec![0x89, b'P', b'N', b'G'],
            }],
        );
        let parts = turn.to_json();
        let parts = parts["content"].as_array().expect("multi-part content");
        assert_eq!(parts.len(), 1, "the image and nothing else: {parts:?}");
        assert_eq!(parts[0]["type"], "image_url");
    }

    /// The bytes decide what a generated picture is called, never the
    /// response — the same rule an upload's magic-number check follows, and
    /// the reason a wrong media type is a grey box on every client.
    #[test]
    fn only_a_png_or_a_jpeg_is_recognised_as_a_picture() {
        assert_eq!(
            sniff_image(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00]),
            Some("image/png")
        );
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        // A WebP, an SVG and an HTML error page a proxy substituted: all
        // refused rather than stored under a guessed type.
        assert_eq!(sniff_image(b"RIFF\x00\x00\x00\x00WEBP"), None);
        assert_eq!(
            sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None
        );
        assert_eq!(sniff_image(b"<!DOCTYPE html>"), None);
        assert_eq!(sniff_image(&[]), None);
    }

    fn route() -> ModelRoute {
        ModelRoute {
            url: "http://provider.invalid/chat".to_string(),
            api_key: "k".to_string(),
            auth: AuthScheme::ApiKey,
            model: "test-gpt-oss".to_string(),
            max_tokens: 1024,
        }
    }

    /// THE REQUEST A SERVER THAT CANNOT DRAW SENDS — byte for byte the one
    /// it sent before tools existed. No `tools` key: not an empty array,
    /// nothing. Every family whose server has no `[ai.images]` is talking
    /// to a text deployment through exactly this body, and "we added a key
    /// to every request for a feature you have not configured" is how a
    /// working assistant stops working.
    #[test]
    fn a_server_that_cannot_draw_declares_no_tools_key_at_all() {
        let body = request_body(&route(), "be brief", &[ChatTurn::user("hello")], &[]);
        assert_eq!(
            body,
            json!({
                "messages": [
                    {"role": "system", "content": "be brief"},
                    {"role": "user", "content": "hello"},
                ],
                "max_tokens": 1024,
                "stream": true,
                "stream_options": {"include_usage": true},
                "model": "test-gpt-oss",
            })
        );
        assert!(
            body.get("tools").is_none(),
            "the key is ABSENT, not empty: {body}"
        );
        assert!(body.get("tool_choice").is_none(), "{body}");
    }

    /// And the one a server that CAN draw sends: the same body with the one
    /// tool declared, pinned field for field so a change to the
    /// declaration is a change to this test.
    #[test]
    fn a_server_that_can_draw_declares_exactly_one_tool() {
        let body = request_body(
            &route(),
            "be brief",
            &[ChatTurn::user("hello")],
            &[draw_picture_tool()],
        );
        assert_eq!(
            body,
            json!({
                "messages": [
                    {"role": "system", "content": "be brief"},
                    {"role": "user", "content": "hello"},
                ],
                "max_tokens": 1024,
                "stream": true,
                "stream_options": {"include_usage": true},
                "model": "test-gpt-oss",
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "draw_picture",
                        "description": "Make a picture for the member. Call this when they ask for a \
                                        picture, a drawing, an image or an illustration, or when a \
                                        picture is plainly the answer they want; answer in words \
                                        otherwise. The image model sees ONLY the prompt you pass — not \
                                        this conversation and not any photograph — so write a complete, \
                                        self-contained description of the picture to make.",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "prompt": {
                                    "type": "string",
                                    "description": "A complete, self-contained description of the \
                                                    picture to make."
                                }
                            },
                            "required": ["prompt"],
                            "additionalProperties": false
                        }
                    }
                }],
            })
        );
        assert!(
            body.get("tool_choice").is_none(),
            "the API's default, auto, IS the rule — the model decides whether: {body}"
        );
    }

    /// A tool call arrives in pieces: the name on the first chunk, the
    /// arguments a few characters at a time, all under one index. They are
    /// joined, and the words — none, for a pure tool call — are untouched.
    #[test]
    fn a_tool_call_is_accumulated_across_deltas() {
        let mut reply = Streamed::default();
        let mut drafts = Vec::new();
        let mut words = String::new();
        let events = [
            json!({"choices": [{"delta": {"role": "assistant", "content": null,
                "tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                "function": {"name": "draw_picture", "arguments": ""}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0,
                "function": {"arguments": "{\"pro"}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0,
                "function": {"arguments": "mpt\": \"a cat in a hat\"}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 40, "completion_tokens": 9}}),
        ];
        for event in &events {
            absorb_event(event, &mut reply, &mut drafts, &mut |delta| {
                words.push_str(delta)
            });
        }
        let reply = finish(reply, drafts);
        assert_eq!(reply.text, "", "a tool call streams no words");
        assert_eq!(words, "", "and nothing reached the audience");
        assert_eq!(
            reply.tool_call,
            Some(ToolCall {
                name: "draw_picture".to_string(),
                arguments: "{\"prompt\": \"a cat in a hat\"}".to_string(),
            })
        );
        assert_eq!(reply.usage.prompt_tokens, 40);
        assert_eq!(reply.usage.completion_tokens, 9);
        assert_eq!(
            reply
                .tool_call
                .expect("the call")
                .draw_prompt(4000)
                .expect("a prompt"),
            "a cat in a hat"
        );
    }

    /// Words AND a call in one stream: both are handed back, and the words
    /// reached `on_delta` as they arrived — which is why the CALLER, not this
    /// file, decides that the picture wins (protocol.md, "Drawing without
    /// being told to").
    #[test]
    fn words_and_a_tool_call_are_both_reported() {
        let mut reply = Streamed::default();
        let mut drafts = Vec::new();
        let mut words = String::new();
        let events = [
            json!({"choices": [{"delta": {"content": "Here you "}}]}),
            json!({"choices": [{"delta": {"content": "are:"}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"function": {"name": "draw_picture",
                "arguments": "{\"prompt\":\"x\"}"}}]}}]}),
        ];
        for event in &events {
            absorb_event(event, &mut reply, &mut drafts, &mut |delta| {
                words.push_str(delta)
            });
        }
        let reply = finish(reply, drafts);
        assert_eq!(reply.text, "Here you are:");
        assert_eq!(words, "Here you are:");
        assert_eq!(
            reply.tool_call.map(|call| call.name),
            Some("draw_picture".to_string()),
            "a missing index is the first call"
        );
    }

    /// A stream with no tool call at all reports none — the shape every text
    /// answer has had since the assistant existed.
    #[test]
    fn a_plain_answer_carries_no_tool_call() {
        let mut reply = Streamed::default();
        let mut drafts = Vec::new();
        absorb_event(
            &json!({"choices": [{"delta": {"content": "cold"}}]}),
            &mut reply,
            &mut drafts,
            &mut |_| {},
        );
        let reply = finish(reply, drafts);
        assert_eq!(reply.text, "cold");
        assert_eq!(reply.tool_call, None);
    }

    /// Every way a call can be wrong is an ERROR, never a guess and never a
    /// silent nothing: the member asked for a picture, the model agreed, and
    /// a blank row that never resolves is the outcome the empty-row design
    /// exists to avoid.
    #[test]
    fn a_bad_draw_call_is_refused_rather_than_repaired() {
        let call = |name: &str, arguments: &str| ToolCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
        };
        assert!(
            call("draw_picture", "{\"prompt\": \"   \"}")
                .draw_prompt(4000)
                .is_err(),
            "blank"
        );
        assert!(
            call("draw_picture", "{}").draw_prompt(4000).is_err(),
            "missing"
        );
        assert!(
            call("draw_picture", "{\"prompt\": 12}")
                .draw_prompt(4000)
                .is_err(),
            "not a string"
        );
        assert!(
            call("draw_picture", "{\"prompt\": \"a cat")
                .draw_prompt(4000)
                .is_err(),
            "not JSON — a stream that ended early"
        );
        assert!(
            call("delete_family", "{\"prompt\": \"a cat\"}")
                .draw_prompt(4000)
                .is_err(),
            "a tool this server never offered"
        );
        assert!(
            call("draw_picture", "{\"prompt\": \"a cat\"}")
                .draw_prompt(5)
                .is_ok(),
            "exactly at the bound"
        );
        assert!(
            call("draw_picture", "{\"prompt\": \"a cat\"}")
                .draw_prompt(4)
                .is_err(),
            "over it — refused, never cut"
        );
        // Characters, not bytes: five Cyrillic letters are five, whatever
        // their encoding, and a bound counted in bytes would refuse a
        // Russian family's prompt at half the length of an English one.
        assert_eq!(
            call("draw_picture", "{\"prompt\": \"  кошка  \"}")
                .draw_prompt(5)
                .expect("five"),
            "кошка"
        );
        assert!(
            call("draw_picture", "{\"prompt\": \"кошка\"}")
                .draw_prompt(4)
                .is_err()
        );
    }

    /// A provider that repeats `function.name` on EVERY chunk — some
    /// OpenAI-compatible proxies do — must not yield `draw_picturedraw_…`,
    /// which the caller would refuse as undeclared and turn every
    /// contextual draw there into `ai_error`. The name is set once; the
    /// arguments still accumulate.
    #[test]
    fn a_name_repeated_on_every_chunk_is_read_once() {
        let mut reply = Streamed::default();
        let mut drafts = Vec::new();
        let events = [
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1",
                "type": "function", "function": {"name": "draw_picture", "arguments": ""}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1",
                "type": "function", "function": {"name": "draw_picture",
                "arguments": "{\"prompt\": \"a "}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1",
                "type": "function", "function": {"name": "draw_picture",
                "arguments": "cat\"}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
        ];
        for event in &events {
            absorb_event(event, &mut reply, &mut drafts, &mut |_| {});
        }
        let reply = finish(reply, drafts);
        assert_eq!(
            reply.tool_call,
            Some(ToolCall {
                name: "draw_picture".to_string(),
                arguments: "{\"prompt\": \"a cat\"}".to_string(),
            })
        );
        assert_eq!(
            reply
                .tool_call
                .expect("the call")
                .draw_prompt(4000)
                .expect("a prompt"),
            "a cat"
        );
    }

    /// A provider indexing past any sane number of calls is dropped, not
    /// allocated for.
    #[test]
    fn a_runaway_tool_index_is_ignored() {
        let mut reply = Streamed::default();
        let mut drafts = Vec::new();
        absorb_event(
            &json!({"choices": [{"delta": {"tool_calls": [{"index": 4_000_000_000_u64,
                "function": {"name": "draw_picture", "arguments": "{}"}}]}}]}),
            &mut reply,
            &mut drafts,
            &mut |_| {},
        );
        assert!(drafts.is_empty(), "{drafts:?}");
    }

    #[test]
    fn a_half_configured_section_counts_as_off() {
        let mut cfg = AiConfig {
            enabled: true,
            endpoint: "https://example.openai.azure.com".to_string(),
            ..Default::default()
        };
        // No deployment, no key: enabled must not be enough to try.
        assert!(!cfg.is_usable());
        cfg.deployment = "d".to_string();
        assert!(!cfg.is_usable());
        cfg.api_key = "k".to_string();
        assert!(cfg.is_usable());
        cfg.enabled = false;
        assert!(!cfg.is_usable());
    }
}
