//! The assistant: Azure OpenAI chat completions, streamed.
//!
//! What leaves this server is deliberately narrow. A request carries the
//! configured system prompt and the last N messages of ONE member's OWN
//! assistant chat — never the family chat, never another member's thread
//! (docs/protocol.md, "The assistant"). That is the invariant which makes a
//! privacy-first family server able to talk to a hosted model at all, and it
//! is enforced here, at the only place that builds a request.
//!
//! Streaming is server-sent events: `data: {json}` lines, terminated by
//! `data: [DONE]`. Parsed by hand rather than with an SSE crate — the format
//! is three rules, and a dependency that owns the parse would also own the
//! reconnect and retry behaviour we deliberately do not want.

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::AiConfig;

/// One turn of the conversation as the API wants it.
#[derive(Debug, Clone, Serialize)]
pub struct ChatTurn {
    pub role: &'static str,
    pub content: String,
}

impl ChatTurn {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant",
            content: content.into(),
        }
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
    cfg: &AiConfig,
    turns: &[ChatTurn],
    mut on_delta: F,
) -> Result<(String, Usage)>
where
    F: FnMut(&str),
{
    let mut messages: Vec<Value> = Vec::with_capacity(turns.len() + 1);
    if !cfg.system_prompt.trim().is_empty() {
        messages.push(json!({"role": "system", "content": cfg.system_prompt}));
    }
    for turn in turns {
        messages.push(json!({"role": turn.role, "content": turn.content}));
    }

    let body = json!({
        "messages": messages,
        "max_tokens": cfg.max_tokens,
        "stream": true,
        // Ask for the token counts in the final chunk; without this Azure
        // sends none when streaming and statistics would have nothing.
        "stream_options": {"include_usage": true},
        "model": cfg.model,
    });

    let response = client
        .post(cfg.completions_url())
        .header("api-key", cfg.api_key.trim())
        .json(&body)
        .send()
        .await
        .context("calling the assistant")?;

    let status = response.status();
    if !status.is_success() {
        // The body may carry the provider's own explanation, which is worth
        // having in the log — but never the key, which is only in a header.
        let detail = response.text().await.unwrap_or_default();
        bail!(
            "assistant returned {status}: {}",
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

#[cfg(test)]
mod tests {
    use super::*;

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
