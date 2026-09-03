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

use crate::config::{AiImagesConfig, ModelRoute};

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

/// Stream a reply, handing each fragment to `on_delta` as it arrives.
///
/// `on_delta` is called on the caller's task, so it should do nothing slow —
/// fanning a frame out to the member's sockets is exactly the right amount
/// of work.
///
/// Returns the full text and what it cost. An error mid-stream still leaves
/// whatever arrived with the caller through `on_delta`; the partial answer
/// is worth more than nothing, and the caller reports it as such.
pub async fn stream_reply<F>(
    client: &reqwest::Client,
    route: &ModelRoute,
    system_prompt: &str,
    turns: &[ChatTurn],
    mut on_delta: F,
) -> Result<(String, Usage)>
where
    F: FnMut(&str),
{
    let mut messages: Vec<Value> = Vec::with_capacity(turns.len() + 1);
    if !system_prompt.trim().is_empty() {
        messages.push(json!({"role": "system", "content": system_prompt}));
    }
    for turn in turns {
        messages.push(turn.to_json());
    }

    let body = json!({
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

    let url = &route.url;
    let response = client
        .post(url)
        .header("api-key", route.api_key.as_str())
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

    let mut text = String::new();
    let mut usage = Usage::default();
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
                return Ok((text, usage));
            }
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                // A fragment we cannot parse is not worth failing a reply
                // over; the next one usually carries on fine.
                continue;
            };
            if let Some(found) = event
                .get("usage")
                .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok())
            {
                usage = found;
            }
            // Azure sends a first chunk with an empty `choices` array when
            // it is only reporting usage, so this is an `if let`, not an
            // index.
            if let Some(delta) = event["choices"]
                .get(0)
                .and_then(|choice| choice["delta"]["content"].as_str())
                && !delta.is_empty()
            {
                text.push_str(delta);
                on_delta(delta);
            }
        }
    }

    Ok((text, usage))
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
/// **The request body is the one contract in this file that could not be
/// verified against a live endpoint.** Its three optional fields are
/// therefore optional in config too, and omitted when empty: `size` and
/// `response_format` are the two that vary between image deployments, and an
/// endpoint that rejects a field it does not implement answers 400 with
/// nothing a family can act on. Both response shapes the OpenAI images
/// contract allows are parsed, whichever one arrives, so asking for neither
/// is the safe default.
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
    if !cfg.response_format.trim().is_empty() {
        body["response_format"] = json!(cfg.response_format.trim());
    }

    let response = client
        .post(&route.url)
        .header("api-key", route.api_key.as_str())
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
/// allocation, not after it. No `api-key` header — the link is pre-signed,
/// and sending the key to whatever host a response named would be handing it
/// over on the provider's say-so.
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
