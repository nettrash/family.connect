//! The assistant: Azure OpenAI chat completions (streamed), image
//! generations, and — since 2026-10-02 — transcriptions.
//!
//! What leaves this server depends on WHERE the question was asked, and the
//! difference has to be stated plainly here: an operator reads this file to
//! decide whether to switch the section on at all.
//!
//! In a member's own assistant chat, a request carries the configured system
//! prompt and the last N messages of THAT MEMBER'S OWN thread — never another
//! member's, and never the family chat (docs/protocol.md, "The assistant").
//!
//! An `@ai` mention IN THE FAMILY CHAT is the other case, and it is not
//! narrow: while the family's `ai_history` is on — and it defaults to ON —
//! recent family conversation goes with the question, other members' words,
//! their display names and their timestamps included, bounded by the history
//! limits (protocol.md, "Mentioning the assistant in the family chat"). Only
//! the family's OWNER can turn that off, and no member is asked first. Both
//! paths are built here, at the only place that builds a request.
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
//! Since 2026-09-30 a description the images deployment REFUSES goes once
//! more to the text deployment, alone under a fixed instruction, to be
//! reworded without real names or brands ([`rephrase_description`]) — the
//! same string to the same provider, and nothing with it (protocol.md, "A
//! refused description is reworded once"). Whether and when that happens is
//! the caller's (`handlers_ai::draw_or_reword`); what is SENT is decided
//! here, like every other request.
//!
//! A TRANSCRIPTION is the narrowest request of all ([`transcribe`]): one
//! recording's sound, the bytes the caller hands over and nothing it went
//! looking for, plus the family's language as a two-letter hint — no prompt,
//! no words, no history (protocol.md, "Transcripts on request"). Whose
//! recording may go, and which bytes, is decided by the caller
//! (`handlers_transcript`), like every other "may this leave" here. It
//! speaks one of two contracts, as configured: Azure OpenAI's
//! `audio/transcriptions`, or Azure Speech's `transcriptions:transcribe`
//! (Microsoft's MAI-Transcribe models), whose `definition` this file writes
//! and which carries the model's name, the style and at most a locale.
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

use crate::config::{AiImagesConfig, AuthScheme, ModelRoute, TranscribeContract, TranscribeRoute};

/// Put the key on the request, in the header this route's provider reads.
///
/// Two schemes because two Azure surfaces disagree: classic Azure OpenAI
/// takes `api-key`, and the Foundry model surface takes an `Authorization:
/// Bearer` — with the SAME key. Which one is CONFIGURED, never sniffed from
/// the URL (see [`AuthScheme`]), so this function only obeys and there is
/// exactly one place in the file where a key meets a request. A third,
/// `Ocp-Apim-Subscription-Key`, is the Azure Speech contract's, and is
/// derived from `[ai.transcribe] api = "speech"` rather than written.
fn with_key(request: reqwest::RequestBuilder, route: &ModelRoute) -> reqwest::RequestBuilder {
    match route.auth {
        AuthScheme::ApiKey => request.header("api-key", route.api_key.as_str()),
        AuthScheme::Bearer => request.bearer_auth(route.api_key.as_str()),
        AuthScheme::SubscriptionKey => {
            request.header("Ocp-Apim-Subscription-Key", route.api_key.as_str())
        }
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

impl Usage {
    /// Two requests' worth, for the one reply that made both — the text
    /// model deciding to draw and the text model rewording a refused
    /// description are two bills against one picture (protocol.md, "Family
    /// statistics"). Saturating, because a provider reporting nonsense must
    /// not panic the reply it already paid for.
    pub fn plus(self, other: Usage) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
        }
    }
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
///
/// And the one fact the model keeps getting wrong without being told: the
/// images deployment's filter refuses a description that NAMES anybody. A
/// prompt written after reading a thread full of family names carried those
/// names, and was refused far more often than a `/draw` (protocol.md,
/// "Drawing without being told to", amended 2026-10-01). So is the other:
/// the member's own description, embellished, was refused where the same
/// words sent as `/draw` were drawn — so the prompt keeps their words.
const DRAW_TOOL_DESCRIPTION: &str = "Make a picture for the member. Call this when they ask for a picture, a drawing, an \
     image or an illustration, or when a picture is plainly the answer they want; answer in words \
     otherwise. The image model sees ONLY the prompt you pass — not this conversation and not any \
     photograph — so write a complete, self-contained description of the picture to make. When the \
     member has described the picture, the prompt is their description in their own words, as close to \
     what they wrote as you can keep it: add only what the conversation makes necessary for it to stand \
     alone, such as what \"it\" refers to, and nothing else — no extra detail, style, mood, age or realism \
     they did not ask for. The image \
     model refuses any prompt that names a person, so never put a name in it — not a family member's, \
     not a first name or a nickname, not a real person's or a public figure's — and never a brand, a \
     logo, or a trademarked or copyrighted character: describe each person by how they look and what \
     they are doing instead, and each thing by what it is.";

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
    /// Why the model stopped, in the provider's own word — `stop`, `length`,
    /// `content_filter`, `tool_calls` — or empty when the stream never said.
    /// It changes nothing the server DOES. It is kept because an answer with
    /// no words in it is otherwise indistinguishable from any other: a reply
    /// cut off by the filter, one that ran out of tokens while reasoning and
    /// one the provider simply returned empty all reach the member as the
    /// same "Couldn't answer that", and used to reach the log as nothing.
    pub finish_reason: String,
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
    let Some(choice) = event["choices"].get(0) else {
        return;
    };
    // The last chunk that names one wins; it arrives once, on the final
    // chunk of a choice, and every chunk before it carries null.
    if let Some(reason) = choice["finish_reason"].as_str() {
        reply.finish_reason = reason.to_string();
    }
    let delta = &choice["delta"];
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

/// The provider's OWN safety filter refused the request — the question, the
/// answer, or a picture's description (docs/protocol.md, "The assistant").
///
/// Carried as the SOURCE of the error a provider call returns, under the
/// usual context line, so every caller keeps the `anyhow` it always had and
/// the one that needs to know asks [`is_refusal`]. It is a type rather than
/// a sentence to search for because the decision is made ONCE, here, from
/// the provider's structured fields — and a caller matching on the words of
/// a log line would be making it a second time, worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the provider's content filter refused it")
    }
}

impl std::error::Error for Refused {}

/// Whether this error is the provider refusing, anywhere in its chain.
pub fn is_refusal(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Refused>())
}

/// `error.code` / `error.type` values that ARE a content refusal. Azure's
/// chat completions say `content_filter`; its images endpoint says
/// `content_policy_violation` or, on the newer surfaces,
/// `content_safety_violation`; OpenAI's image models say `moderation_blocked`,
/// and the same models served by Azure (`gpt-image-*`, documented 2026) say
/// `contentFilter` — camel case, which no case-insensitive match against
/// `content_filter` catches, so it is listed in its own spelling.
const REFUSAL_CODES: &[&str] = &[
    "content_filter",
    "contentFilter",
    "content_policy_violation",
    "content_safety_violation",
    "moderation_blocked",
];

/// `error.innererror.code` values that are one — Azure's name for "the RAI
/// policy blocked this", under whatever outer code the surface chose.
const REFUSAL_INNER_CODES: &[&str] = &["ResponsibleAIPolicyViolation"];

/// Outer codes that say only "the request was bad". They leave the question
/// open, so the message is allowed to answer it — and ONLY they do: a code
/// that names some other problem is that problem, whatever its prose says.
const GENERIC_CODES: &[&str] = &["badrequest", "bad_request", "invalid_request_error"];

/// Phrases a message may name the policy by, consulted only under a generic
/// or absent code. Each is Azure naming its own filter outright; none is a
/// word a member's question could put into an unrelated error.
const REFUSAL_PHRASES: &[&str] = &[
    "responsibleaipolicyviolation",
    "rai policy",
    "content management policy",
];

/// The error object of a provider's error body. Every Azure and OpenAI
/// surface wraps it in `error`; a bare object is read as the error itself
/// rather than as nothing.
fn error_object(parsed: &Value) -> &Value {
    if parsed["error"].is_object() {
        &parsed["error"]
    } else {
        parsed
    }
}

/// Did the provider's filter refuse this request? Decided from an HTTP error
/// answer: its status and its body, as the provider sent them.
///
/// **Structured fields decide.** A 4xx whose JSON `error.code`, `error.type`
/// or `error.innererror.code` names a refusal is one; a 4xx that names
/// anything else — `max_tokens` too large, an unknown deployment — is not,
/// whatever it says. Only when those fields are absent or merely generic
/// does the message count, and then only a phrase that names Azure's policy
/// outright. A 5xx is never a refusal: the provider did not read the request
/// and decline it, it failed to answer, and asking again may well work.
pub fn refused_by_provider(status: reqwest::StatusCode, body: &str) -> bool {
    if !status.is_client_error() {
        return false;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let error = error_object(&parsed);
    let is_one_of = |value: &Value, set: &[&str]| {
        value
            .as_str()
            .is_some_and(|found| set.iter().any(|known| found.eq_ignore_ascii_case(known)))
    };
    let outer = [&error["code"], &error["type"]];
    if outer.iter().any(|value| is_one_of(value, REFUSAL_CODES)) {
        return true;
    }
    let inner = [&error["innererror"]["code"], &error["inner_error"]["code"]];
    if inner
        .iter()
        .any(|value| is_one_of(value, REFUSAL_INNER_CODES) || is_one_of(value, REFUSAL_CODES))
    {
        return true;
    }
    // A code that is a string and neither a refusal nor generic has said
    // what went wrong, and it was something else.
    let named_something_else = outer.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|found| !found.trim().is_empty() && !is_one_of(value, GENERIC_CODES))
    });
    if named_something_else {
        return false;
    }
    let message = error["message"].as_str().unwrap_or_default().to_lowercase();
    REFUSAL_PHRASES
        .iter()
        .any(|phrase| message.contains(phrase))
}

/// Did a streamed answer stop because the provider's filter stopped it?
/// Azure's word for it, on the last chunk of the choice.
pub fn finish_is_refusal(finish_reason: &str) -> bool {
    finish_reason.eq_ignore_ascii_case("content_filter")
}

/// Text for the log as ONE line, bounded — what [`loggable_detail`] kept of
/// a provider's error body.
///
/// Azure pretty-prints its errors, and journald cuts a record at every
/// newline — so the part of the detail that says WHICH filter tripped was
/// arriving as a second, orphaned line, or not at all. Every run of
/// whitespace becomes one space, then the 400-character bound applies,
/// counted in characters so a body in any alphabet is never cut mid-letter.
fn one_line(detail: &str) -> String {
    detail
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}

/// A provider's error body as the log may keep it: the fields that NAME the
/// failure, and nothing that can carry the member's words.
///
/// **The body itself is never logged.** An error answer can repeat what it
/// was sent: a DALL·E 3 refusal may carry the model's `revised_prompt` of
/// the member's description, a pydantic-style 422 echoes the request under
/// `detail[].input`, and a validation `message` can quote the value it
/// rejected. So this is an allow-list, not a filter — `error.code`,
/// `error.type`, `error.param`, the inner error's `code`, the content-filter
/// categories that tripped (with their severity), and a 422's `loc`/`type`
/// pairs — each kept only while it looks like the identifier it is, and
/// everything else, `message` included, counted and withheld. What is left
/// is folded by [`one_line`], so it keeps the 400-character bound.
fn loggable_detail(body: &str) -> String {
    let bytes = body.len();
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return format!("{bytes}-byte body, not JSON, withheld");
    };
    let error = error_object(&parsed);
    // Azure spells it three ways: `innererror` (OpenAI surfaces),
    // `inner_error`, and `innerError` (the Speech contract).
    let inner = ["innererror", "innerError", "inner_error"]
        .iter()
        .map(|key| &error[*key])
        .find(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let mut parts = Vec::new();
    for (label, value) in [
        ("code", &error["code"]),
        ("type", &error["type"]),
        ("param", &error["param"]),
        ("inner", &inner["code"]),
    ] {
        if let Some(token) = log_token(value) {
            parts.push(format!("{label}={token}"));
        }
    }
    // Which of the filter's categories said no — Azure spells the key both
    // ways, and puts it under the inner error or beside the code.
    let mut filtered = Vec::new();
    for holder in [inner, error] {
        for key in ["content_filter_result", "content_filter_results"] {
            let Some(categories) = holder[key].as_object() else {
                continue;
            };
            for (category, result) in categories {
                if result["filtered"] != Value::Bool(true) {
                    continue;
                }
                let Some(category) = log_token(&Value::from(category.as_str())) else {
                    continue;
                };
                filtered.push(match log_token(&result["severity"]) {
                    Some(severity) => format!("{category}:{severity}"),
                    None => category,
                });
            }
        }
    }
    if !filtered.is_empty() {
        parts.push(format!("filtered={}", filtered.join(",")));
    }
    // A pydantic-style 422 names the field and the rule, and echoes the
    // input beside them; the first two are kept.
    if let Some(items) = parsed["detail"].as_array() {
        let invalid: Vec<String> = items
            .iter()
            .take(4)
            .filter_map(|item| {
                let loc = item["loc"]
                    .as_array()?
                    .iter()
                    .map(log_token)
                    .collect::<Option<Vec<_>>>()?
                    .join(".");
                let rule = log_token(&item["type"]).unwrap_or_else(|| "?".to_string());
                Some(format!("{loc}:{rule}"))
            })
            .collect();
        if !invalid.is_empty() {
            parts.push(format!("invalid={}", invalid.join(",")));
        }
    }
    parts.push(format!("{bytes}-byte body, other fields withheld"));
    one_line(&parts.join(" "))
}

/// One field of a provider's error, if it is shaped like an identifier —
/// a short run of ASCII letters, digits and `_-.[]:` — or a number. A
/// value of any other shape is prose, and prose is where a provider
/// repeats what it was sent, so it is not kept.
fn log_token(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let identifier = !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.[]:".contains(c));
    identifier.then_some(text)
}

/// The error a failed provider call becomes: the context line the log
/// reads, over [`Refused`] when the provider's filter is what failed it.
fn provider_error(status: reqwest::StatusCode, summary: String, detail: &str) -> anyhow::Error {
    if refused_by_provider(status, detail) {
        anyhow::Error::new(Refused).context(summary)
    } else {
        anyhow::anyhow!(summary)
    }
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
        // The body is read for WHY as well as for the log: a refusal by
        // the provider's filter is the one failure the member is told about
        // differently (protocol.md, "The assistant"). Only its identifying
        // fields reach the log: an error can echo the question it refused.
        let detail = response.text().await.unwrap_or_default();
        return Err(provider_error(
            status,
            format!(
                "assistant returned {status} for {url}: {}",
                loggable_detail(&detail)
            ),
            &detail,
        ));
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
        // None of it is secret — the key is only ever a header. The body
        // is not logged whole: a refusal can repeat the description.
        let detail = response.text().await.unwrap_or_default();
        return Err(provider_error(
            status,
            format!(
                "image generation returned {status} for {}: {}",
                route.url,
                loggable_detail(&detail)
            ),
            &detail,
        ));
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

/// One recording, ready to travel to the transcription deployment.
///
/// Bytes, a media type and a file NAME — and the name is not decoration: the
/// OpenAI transcription surface reads a file's FORMAT from its extension, so
/// an `.m4a` sent as `file.bin` is refused as an unsupported format. The
/// caller picks both from the attachment's stored type (or from the one
/// shape a device may supply), so nothing here guesses.
#[derive(Debug, Clone)]
pub struct Recording {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub filename: &'static str,
}

/// What came back: the words, and the language when the provider named one.
///
/// `text` may be EMPTY — silence is an answer ("No speech"), not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
}

/// The most bytes a transcription ANSWER may be. Fifty minutes of speech is
/// tens of kilobytes of text; this bounds the read against a runaway or
/// misdirected response, the way `max_bytes` bounds a picture.
const TRANSCRIPT_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// The family's language as a transcription HINT: the bare ISO 639-1
/// language, with the script and anything else after the first `-` dropped
/// (`sr-Latn` → `sr`, `zh-Hans` → `zh`).
///
/// A speech model hears a language, not an alphabet, and the provider's
/// `language` field takes the two-letter code. `None` for anything that is
/// not two or three ASCII letters, so a value this server never stored can
/// never be sent (protocol.md, "The family's language").
pub fn transcription_language(family_language: &str) -> Option<String> {
    let language = family_language
        .trim()
        .split('-')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let shaped =
        (2..=3).contains(&language.len()) && language.chars().all(|c| c.is_ascii_lowercase());
    shaped.then_some(language)
}

/// What a provider call came back with: the transcript, and the length of
/// sound the provider says it heard.
///
/// Every one is an ANSWER, kept like any other: a provider refusal is never
/// turned into one, `""` included — see [`Unreadable`] and [`Unheard`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcription {
    pub transcript: Transcript,
    /// The provider's `durationMilliseconds`, when it gave one. The usage
    /// row prefers it to the attachment's own length.
    pub audio_ms: Option<i64>,
}

/// The provider refused the FILE — its format, or its length — rather than
/// failing to answer: the client is told `not_transcribable` and may send a
/// sound track of its own. Carried in the error's chain like [`Refused`],
/// and decided once, from the provider's structured codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the provider could not read this recording")
    }
}

impl std::error::Error for Unreadable {}

/// Whether this error is the provider refusing the file, anywhere in its
/// chain.
pub fn is_unreadable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Unreadable>())
}

/// The provider could not tell which language it heard (the speech
/// contract's `NoLanguageIdentified`). It is what a language the model does
/// not know looks like — Serbian on MAI-Transcribe-2 — so it is NOT
/// silence: answered `""`, it would be drawn as "No speech" and kept, by the
/// server and by the asker's device, for a recording somebody spoke in.
/// The client is told `not_transcribable`, like [`Unreadable`], and the log
/// says `unheard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unheard;

impl std::fmt::Display for Unheard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the provider could not tell which language this recording is in")
    }
}

impl std::error::Error for Unheard {}

/// Whether this error is the provider naming no language, anywhere in its
/// chain.
pub fn is_unheard(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Unheard>())
}

/// The family languages the speech contract is never sent as a locale, even
/// with `language_hint` on: MAI-Transcribe-2 does not list them, and a
/// locale it does not know is refused (`UnsupportedLanguageCode`) rather
/// than ignored. Compared against the BARE language, so `sr-Latn` is `sr`.
const SPEECH_LOCALES_NEVER_SENT: &[&str] = &["sr"];

/// The language hint this request may carry, from the family's (already
/// shaped by [`transcription_language`]): none unless the route allows a
/// hint, and under the speech contract never one it would refuse.
fn hint_for(route: &TranscribeRoute, language: Option<&str>) -> Option<String> {
    if !route.language_hint {
        return None;
    }
    let language = language?;
    match route.contract {
        TranscribeContract::OpenAi => Some(language.to_string()),
        TranscribeContract::Speech { .. } => {
            let bare = language.split('-').next().unwrap_or_default();
            (!SPEECH_LOCALES_NEVER_SENT
                .iter()
                .any(|never| bare.eq_ignore_ascii_case(never)))
            .then(|| bare.to_ascii_lowercase())
        }
    }
}

/// The speech contract's `definition` part — the whole of what goes beside
/// the sound: the model, the style, and at most ONE locale. Its own
/// function so a test can pin it to the byte.
fn speech_definition(model: &str, style: &str, locale: Option<&str>) -> Value {
    let mut definition = json!({
        "enhancedMode": {
            "enabled": true,
            "model": model,
            "modelOptions": {"transcribeStyle": style},
        },
    });
    if let Some(locale) = locale {
        definition["locales"] = json!([locale]);
    }
    definition
}

/// Ask the transcription deployment for the text of ONE recording, in the
/// contract the route names.
///
/// **`openai`** — what leaves, and the whole of it: the recording's bytes
/// as `file`, `response_format=json`, the `model` field every request here
/// carries (the DEPLOYMENT name — the v1 surface routes on it, the classic
/// surface ignores it), and `language` when the caller has a hint. Azure's
/// documented contract (Microsoft Learn, "Speech to text with transcription
/// models"); like the images surface it is confirmed against a live
/// endpoint by the operator, not from here. A refusal by the provider's
/// filter comes back as [`Refused`] in the error's chain, decided by
/// [`refused_by_provider`] exactly as for every other request.
///
/// **`speech`** — the recording's bytes as `audio` and the
/// [`speech_definition`], nothing else; the key in
/// `Ocp-Apim-Subscription-Key`. Its refusals are read by
/// [`speech_failure`]: a refused FILE is [`Unreadable`], an empty one or an
/// unidentified language is an answer of silence, and nothing is ever
/// [`Refused`] — that contract documents no filter.
///
/// The error never carries the text: an error body is logged only through
/// [`loggable_detail`].
pub async fn transcribe(
    client: &reqwest::Client,
    route: &TranscribeRoute,
    recording: Recording,
    language: Option<&str>,
) -> Result<Transcription> {
    let hint = hint_for(route, language);
    let model = &route.route;
    let file = reqwest::multipart::Part::bytes(recording.bytes)
        .file_name(recording.filename)
        .mime_str(recording.mime)
        .context("naming the recording's media type")?;
    let form = match route.contract {
        TranscribeContract::OpenAi => {
            let mut form = reqwest::multipart::Form::new()
                .part("file", file)
                .text("model", model.model.clone())
                .text("response_format", "json");
            if let Some(language) = hint {
                form = form.text("language", language);
            }
            form
        }
        TranscribeContract::Speech { style } => {
            let definition = speech_definition(&model.model, style.as_str(), hint.as_deref());
            reqwest::multipart::Form::new()
                .part("audio", file)
                .text("definition", definition.to_string())
        }
    };

    let response = with_key(client.post(&model.url), model)
        .multipart(form)
        .send()
        .await
        .context("asking for a transcript")?;

    let status = response.status();
    if !status.is_success() {
        // The URL in the message for the reason it is everywhere here; the
        // body only through the allow-list, because an error can repeat
        // what it was sent — and what it was sent is somebody's voice.
        let detail = response.text().await.unwrap_or_default();
        let summary = format!(
            "transcription returned {status} for {}: {}",
            model.url,
            loggable_detail(&detail)
        );
        return match route.contract {
            TranscribeContract::OpenAi => Err(provider_error(status, summary, &detail)),
            TranscribeContract::Speech { .. } => match speech_failure(status, &detail) {
                SpeechFailure::Unheard => Err(anyhow::Error::new(Unheard).context(summary)),
                SpeechFailure::Unreadable => Err(anyhow::Error::new(Unreadable).context(summary)),
                SpeechFailure::Failed => Err(anyhow::anyhow!(summary)),
            },
        };
    }

    let mut body: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the transcript")?;
        if body.len() + chunk.len() > TRANSCRIPT_MAX_RESPONSE_BYTES {
            bail!("transcript is over the {TRANSCRIPT_MAX_RESPONSE_BYTES} bytes the server reads");
        }
        body.extend_from_slice(&chunk);
    }
    match route.contract {
        TranscribeContract::OpenAi => Ok(Transcription {
            transcript: parse_transcript(&body)?,
            audio_ms: None,
        }),
        TranscribeContract::Speech { .. } => {
            let (transcript, audio_ms) = parse_speech_transcript(&body)?;
            Ok(Transcription {
                transcript,
                audio_ms,
            })
        }
    }
}

/// A language as a client may be handed it: letters and `-`, at most 32 —
/// a name (`russian`), a code (`ru`) or a locale (`en-US`). Anything else
/// is prose, and prose has no business there.
fn shaped_language(language: &str) -> Option<String> {
    let language = language.trim();
    let shaped = !language.is_empty()
        && language.len() <= 32
        && language
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-');
    shaped.then(|| language.to_string())
}

/// The `json` response: `{"text": "…"}`, sometimes with a `language`.
///
/// `text` is required — a 200 without it is a deployment answering some
/// other contract, and a failure rather than silence. `language` is kept
/// only while it is shaped like a language name or code (letters, `-`, at
/// most 32): it is handed to a client, and prose has no business there.
fn parse_transcript(body: &[u8]) -> Result<Transcript> {
    let parsed: Value = serde_json::from_slice(body).context("the transcript was not JSON")?;
    let Some(text) = parsed["text"].as_str() else {
        bail!("transcription response carried no text");
    };
    let language = parsed["language"].as_str().and_then(shaped_language);
    Ok(Transcript {
        text: text.trim().to_string(),
        language,
    })
}

/// The speech contract's 200: `{"durationMilliseconds", "combinedPhrases":
/// [{"text", "channel"?}], "phrases": [{"text", "locale", …}]}`.
///
/// The text is each `combinedPhrases` entry's, in CHANNEL order (an entry
/// without one is channel 0, and entries of one channel keep the order
/// they came in), one line per entry; none, or only empty ones, is `""` —
/// silence, an answer. The language is the first phrase's `locale`, shaped
/// as [`shaped_language`] requires. The duration is
/// `durationMilliseconds`, when it is a non-negative number.
///
/// `combinedPhrases` is documented as always there. When it is missing (or
/// holds no text) while `phrases` holds some, the text is the phrases'
/// own — joined with a space within a channel, a line between channels —
/// because what was said must never be read as silence.
///
/// A 200 that carries NEITHER list is not silence: it is a URL answering
/// some other contract, or an answer with its text missing, and kept as
/// `""` it would tell every later asker that nothing was said. So it is a
/// failure.
fn parse_speech_transcript(body: &[u8]) -> Result<(Transcript, Option<i64>)> {
    let parsed: Value = serde_json::from_slice(body).context("the transcript was not JSON")?;
    let combined = parsed["combinedPhrases"].as_array();
    let phrases = parsed["phrases"].as_array();
    if combined.is_none() && phrases.is_none() {
        bail!("transcription response is not the speech contract's");
    }
    let mut text = combined
        .map(|entries| channel_text(entries, "\n"))
        .unwrap_or_default();
    if text.is_empty() {
        text = phrases
            .map(|entries| channel_text(entries, " "))
            .unwrap_or_default();
    }
    let language = phrases
        .and_then(|phrases| phrases.first())
        .and_then(|phrase| phrase["locale"].as_str())
        .and_then(shaped_language);
    let audio_ms = parsed["durationMilliseconds"]
        .as_i64()
        .or_else(|| {
            parsed["durationMilliseconds"]
                .as_f64()
                .filter(|ms| ms.is_finite() && *ms < i64::MAX as f64)
                .map(|ms| ms.round() as i64)
        })
        .filter(|ms| *ms >= 0);
    Ok((Transcript { text, language }, audio_ms))
}

/// The non-empty texts of a list of the speech contract's phrases, by
/// CHANNEL (missing is 0; stable, so one channel keeps its order): the
/// entries of one channel joined by `within`, channels by a line.
fn channel_text(entries: &[Value], within: &str) -> String {
    let mut texts: Vec<(i64, &str)> = entries
        .iter()
        .filter_map(|entry| {
            let text = entry["text"].as_str()?.trim();
            let channel = entry["channel"].as_i64().unwrap_or(0);
            (!text.is_empty()).then_some((channel, text))
        })
        .collect();
    texts.sort_by_key(|(channel, _)| *channel);
    let mut out = String::new();
    let mut last = None;
    for (channel, text) in texts {
        if let Some(previous) = last {
            out.push_str(if previous == channel { within } else { "\n" });
        }
        out.push_str(text);
        last = Some(channel);
    }
    out
}

/// What a refusal by the speech contract means to a member. None of them
/// is an answer: a refusal is never turned into `""`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechFailure {
    /// The FILE was refused: format, length, or no audio found in bytes
    /// this server checked are there — `not_transcribable`.
    Unreadable,
    /// No language identified — `not_transcribable` too (see [`Unheard`]).
    Unheard,
    /// Anything else — `internal`, transient.
    Failed,
}

/// Inner codes that refuse the FILE. `EmptyAudioFile` is one: the server
/// never sends empty bytes (an upload, and a supplied part, are checked to
/// be non-empty), so it can only mean the provider found no audio it could
/// read in a file that has some — a container it does not read, which the
/// client's own sound track may get round.
const SPEECH_UNREADABLE_INNER: &[&str] = &[
    "InvalidAudioFormat",
    "AudioLengthLimitExceeded",
    "EmptyAudioFile",
];
/// Outer codes that do.
const SPEECH_UNREADABLE_OUTER: &[&str] = &["UnsupportedMediaType"];

/// Read the speech contract's error answer: `{"code", "message",
/// "innerError": {"code", "message"}}` — the codes alone decide, never a
/// message.
///
/// Only a 4xx is read: a 5xx is the provider failing to answer, whatever
/// its body says, and asking again may well work. `TooManyRequests` is a
/// 429, and lands with every other code nobody listed, in `Failed`.
fn speech_failure(status: reqwest::StatusCode, body: &str) -> SpeechFailure {
    if !status.is_client_error() {
        return SpeechFailure::Failed;
    }
    if status == reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
        || status == reqwest::StatusCode::PAYLOAD_TOO_LARGE
    {
        return SpeechFailure::Unreadable;
    }
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = error_object(&parsed);
    let inner = ["innerError", "innererror", "inner_error"]
        .iter()
        .map(|key| &error[*key]["code"])
        .find_map(Value::as_str)
        .unwrap_or_default();
    let outer = error["code"].as_str().unwrap_or_default();
    let is = |found: &str, set: &[&str]| set.iter().any(|known| found.eq_ignore_ascii_case(known));
    if is(inner, SPEECH_UNREADABLE_INNER) || is(outer, SPEECH_UNREADABLE_OUTER) {
        return SpeechFailure::Unreadable;
    }
    if inner.eq_ignore_ascii_case("NoLanguageIdentified") {
        return SpeechFailure::Unheard;
    }
    SpeechFailure::Failed
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

/// What the text deployment is told when it is asked to reword a refused
/// description — the whole of the system prompt on that request.
///
/// The server's own words and never the operator's configured prompt: that
/// one is about answering a family, and this request answers nobody. It
/// names what the images filter refuses, says what to keep, and asks for
/// the rewrite ALONE, because whatever comes back is sent to the images
/// deployment verbatim — "Sure! Here is the description:" would be drawn.
/// It asks for the description's own language to be kept, because a
/// `/draw` goes as written and a rewrite is not a translation (protocol.md,
/// "Asking for a picture").
pub const REPHRASE_INSTRUCTION: &str = "You reword descriptions of pictures for an image generator \
     that refuses any description naming a real person, a public figure, a brand, a logo, or a trademarked \
     or copyrighted character. Every person's name counts, a first name or a nickname included. Rewrite \
     the description you are given so that it keeps everything that is \
     to be drawn — the subjects, the scene, the style and the mood — but names none of those: describe each \
     of them in general words instead, by how they look and what they are doing, never by name. Keep the \
     language the description is written in. Answer with ONLY the rewritten description — no quotes, no \
     explanation, nothing before or after it.";

/// The request that rewords a refused description: the fixed instruction,
/// and the description as the one user turn.
///
/// That is the whole of it, and the reason it is its own function is so a
/// test can pin it: no thread, no transcript, no member's name, no language
/// line, no picture and no tool — the string the images deployment was just
/// sent, going to the same provider's text deployment, and nothing with it
/// (protocol.md, "A refused description is reworded once").
fn rephrase_request(description: &str) -> (&'static str, [ChatTurn; 1]) {
    (REPHRASE_INSTRUCTION, [ChatTurn::user(description)])
}

/// Check a rewrite before it may leave for the images deployment.
///
/// Held to a draw prompt's bounds — trimmed, not blank, at most `max_chars`
/// characters (the message-body ceiling, as for `ToolCall::draw_prompt`) —
/// and refused rather than repaired, like every other prompt. One more:
/// a rewrite that is the description again would be the same refusal
/// bought twice, so it is not sent.
fn checked_rewrite(rewrite: &str, description: &str, max_chars: usize) -> Result<String> {
    let rewrite = rewrite.trim();
    if rewrite.is_empty() {
        bail!("the rewrite came back empty");
    }
    // Characters, never bytes, for the reason `draw_prompt` gives.
    let chars = rewrite.chars().count();
    if chars > max_chars {
        bail!("the rewrite is {chars} characters, over the {max_chars} allowed");
    }
    if rewrite == description.trim() {
        bail!("the rewrite is the description unchanged");
    }
    Ok(rewrite.to_string())
}

/// The rewrite a finished stream carries, or why there is none.
///
/// A rewrite is drawn only when the model SAID it had finished it
/// (`finish_reason: "stop"`). Any other ending leaves a fragment, and a
/// fragment is not a description: the filter stopping it (`content_filter`)
/// is a refusal whatever words had streamed by then — "a blonde pop singer"
/// cut off mid-sentence would be drawn as though it were the whole of what
/// was meant — and the token ceiling (`length`, which a reasoning model can
/// reach before it has written much), a stream that never said, or any
/// other word is a rewrite that did not arrive. The chat path holds the
/// same line for a tool call the filter ended ("a call the provider cut off
/// is a fragment"). Checked BEFORE [`checked_rewrite`], because a fragment
/// passes every one of its checks. The error names the finish reason — the
/// provider's own word for what went wrong — and never the words.
fn finished_rewrite(streamed: &Streamed, description: &str, max_chars: usize) -> Result<String> {
    if finish_is_refusal(&streamed.finish_reason) {
        return Err(anyhow::Error::new(Refused).context("the rewrite was filtered"));
    }
    if streamed.finish_reason != "stop" {
        let reason = if streamed.finish_reason.is_empty() {
            "(none given)"
        } else {
            streamed.finish_reason.as_str()
        };
        bail!("the rewrite did not finish (finish_reason {reason})");
    }
    checked_rewrite(&streamed.text, description, max_chars)
}

/// Ask the TEXT deployment, once, to reword a description the images
/// deployment refused, and hand back the rewrite and what it cost.
///
/// `route` is the text route (`[ai]`): the same provider the member already
/// agreed to, and the request is [`rephrase_request`] and nothing else. It
/// streams like every other text request — one request shape, one parser —
/// but nobody is streamed to: the words are the server's to check, not a
/// reply. A tool call cannot come back, because none is declared.
///
/// Every way this can fail is an error, and the caller turns every one of
/// them into the refusal the member would have had without it: the request
/// failing or being refused, an answer the model did not finish — the filter
/// or the token ceiling ending it, whatever it had said by then
/// ([`finished_rewrite`]) — and a rewrite [`checked_rewrite`] turns away.
/// Neither the description nor the rewrite is in any error this returns — a
/// provider's error is logged by its identifying fields alone
/// ([`loggable_detail`]), and the checks above name a finish reason or a
/// count, never words.
pub async fn rephrase_description(
    client: &reqwest::Client,
    route: &ModelRoute,
    description: &str,
    max_chars: usize,
) -> Result<(String, Usage)> {
    let (instruction, turns) = rephrase_request(description);
    let streamed = stream_reply(client, route, instruction, &turns, &[], |_| {}).await?;
    let rewrite = finished_rewrite(&streamed, description, max_chars)?;
    Ok((rewrite, streamed.usage))
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
            processor: "Microsoft — Azure OpenAI".to_string(),
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
            processor: "Microsoft — Azure OpenAI".to_string(),
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
            processor: "Microsoft — Azure OpenAI".to_string(),
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
                                        self-contained description of the picture to make. When the \
                                        member has described the picture, the prompt is their \
                                        description in their own words, as close to what they wrote as \
                                        you can keep it: add only what the conversation makes necessary \
                                        for it to stand alone, such as what \"it\" refers to, and \
                                        nothing else — no extra detail, style, mood, age or realism they \
                                        did not ask for. The image \
                                        model refuses any prompt that names a person, so never put a \
                                        name in it — not a family member's, not a first name or a \
                                        nickname, not a real person's or a public figure's — and never \
                                        a brand, a logo, or a trademarked or copyrighted character: \
                                        describe each person by how they look and what they are doing \
                                        instead, and each thing by what it is.",
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

    /// WHY the model stopped is kept, because it is the only thing that tells
    /// an empty answer apart from any other: the filter, a reasoning model
    /// out of tokens and a provider that returned nothing all stream no words.
    /// It rides on the last chunk of a choice, null on every chunk before,
    /// and the usage-only chunk after it must not wipe it.
    #[test]
    fn the_reason_a_reply_stopped_is_kept() {
        for (reason, completion) in [("content_filter", 0), ("length", 16384), ("stop", 12)] {
            let mut reply = Streamed::default();
            let mut drafts = Vec::new();
            let events = [
                json!({"choices": [{"delta": {"role": "assistant"}, "finish_reason": null}]}),
                json!({"choices": [{"delta": {}, "finish_reason": reason}]}),
                json!({"choices": [], "usage": {"prompt_tokens": 40, "completion_tokens": completion}}),
            ];
            for event in &events {
                absorb_event(event, &mut reply, &mut drafts, &mut |_| {});
            }
            let reply = finish(reply, drafts);
            assert_eq!(reply.text, "", "no words arrived");
            assert_eq!(reply.finish_reason, reason);
            assert_eq!(reply.usage.completion_tokens, completion);
        }
        // A stream that never names one leaves it empty rather than guessing.
        let mut reply = Streamed::default();
        absorb_event(
            &json!({"choices": [{"delta": {"content": "hi"}}]}),
            &mut reply,
            &mut Vec::new(),
            &mut |_| {},
        );
        assert_eq!(reply.finish_reason, "");
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

    // -- rewording a refused description ---------------------------------------

    /// WHAT LEAVES on a rewrite, pinned field for field: the server's fixed
    /// instruction as the system turn, the description as the one user turn,
    /// and the body every text request has — no tool, no history, no
    /// language line, no picture (protocol.md, "A refused description is
    /// reworded once").
    #[test]
    fn a_rewrite_request_is_the_description_and_nothing_else() {
        let description = "Taylor Swift singing to our cat";
        let (instruction, turns) = rephrase_request(description);
        let body = request_body(&route(), instruction, &turns, &[]);
        assert_eq!(
            body,
            json!({
                "messages": [
                    {"role": "system", "content": REPHRASE_INSTRUCTION},
                    {"role": "user", "content": "Taylor Swift singing to our cat"},
                ],
                "max_tokens": 1024,
                "stream": true,
                "stream_options": {"include_usage": true},
                "model": "test-gpt-oss",
            })
        );
        assert!(body.get("tools").is_none(), "no tool is declared: {body}");
        assert!(
            !body.to_string().contains("data:image"),
            "no picture travels: {body}"
        );
    }

    /// The tool tells the model, before it writes a prompt, what the images
    /// filter refuses: a name of anybody — a family member's above all, since
    /// the model has just read a thread full of them — and the brands and
    /// characters the rewrite would otherwise have to take out afterwards
    /// (protocol.md, "Drawing without being told to", amended 2026-10-01).
    #[test]
    fn the_tool_says_a_prompt_names_nobody() {
        let tool = draw_picture_tool();
        let description = tool["function"]["description"].as_str().unwrap();
        for named in [
            "self-contained",
            "in their own words",
            "nothing else — no extra detail",
            "never put a name in it",
            "family member",
            "first name or a nickname",
            "public figure",
            "brand",
            "trademarked or copyrighted character",
            "how they look and what they are doing",
        ] {
            assert!(
                description.contains(named),
                "the tool must say {named:?}: {description}"
            );
        }
    }

    /// The instruction names every kind of thing the images filter refuses,
    /// asks for the rewrite alone — whatever comes back is drawn verbatim —
    /// and keeps the description's language, because a rewrite is not a
    /// translation.
    #[test]
    fn the_rewrite_instruction_says_what_to_drop_what_to_keep_and_what_to_answer() {
        for named in [
            "real person",
            "public figure",
            "brand",
            "trademarked",
            "copyrighted character",
            "first name or a nickname",
            "general words",
            "keeps everything that is to be drawn",
            "Keep the language",
            "ONLY the rewritten description",
        ] {
            assert!(
                REPHRASE_INSTRUCTION.contains(named),
                "the instruction must say {named:?}: {REPHRASE_INSTRUCTION}"
            );
        }
        // It is the server's own words, never the operator's prompt, and it
        // is not an instruction about answering anybody.
        assert!(!REPHRASE_INSTRUCTION.contains("family"));
    }

    /// A rewrite is held to a draw prompt's bounds — trimmed, not blank,
    /// counted in characters — and refused rather than cut. The description
    /// handed back unchanged is refused too: it would be the same refusal
    /// bought twice.
    #[test]
    fn a_rewrite_is_held_to_a_draw_prompts_bounds() {
        let description = "Pikachu at Anna's birthday";
        assert_eq!(
            checked_rewrite(
                "  a small yellow cartoon creature at a birthday party \n",
                description,
                4000
            )
            .expect("an ordinary rewrite"),
            "a small yellow cartoon creature at a birthday party"
        );
        assert!(checked_rewrite("", description, 4000).is_err(), "empty");
        assert!(
            checked_rewrite(" \n\t", description, 4000).is_err(),
            "blank"
        );
        assert!(
            checked_rewrite(" Pikachu at Anna's birthday ", description, 4000).is_err(),
            "unchanged"
        );
        assert!(
            checked_rewrite("a cat", description, 5).is_ok(),
            "at the bound"
        );
        assert!(
            checked_rewrite("a cat", description, 4).is_err(),
            "over it — refused, never cut"
        );
        // Five Cyrillic letters are five, whatever their encoding.
        assert_eq!(
            checked_rewrite("кошка", description, 5).expect("five"),
            "кошка"
        );
        assert!(checked_rewrite("кошка", description, 4).is_err());
        // What an error says is a count, never the words.
        let error = format!(
            "{:#}",
            checked_rewrite("a lilac hedgehog", description, 3).expect_err("over")
        );
        assert!(!error.contains("hedgehog"), "{error}");
        let error = format!(
            "{:#}",
            checked_rewrite(description, description, 4000).expect_err("unchanged")
        );
        assert!(!error.contains("Pikachu"), "{error}");
    }

    /// Only a rewrite the model FINISHED is drawn. Words the filter cut
    /// short are a refusal however many of them streamed first, and words
    /// cut off by the token ceiling — or by a stream that never said why it
    /// stopped — are no rewrite at all. Before this the filter counted only
    /// when nothing had been written, and "a blonde pop singer", ended by
    /// the filter mid-sentence, passed every check and was drawn.
    #[test]
    fn a_rewrite_the_model_did_not_finish_is_never_drawn() {
        let description = "Taylor Swift singing to our cat";
        let ended = |text: &str, finish_reason: &str| Streamed {
            text: text.to_string(),
            finish_reason: finish_reason.to_string(),
            ..Default::default()
        };

        assert_eq!(
            finished_rewrite(
                &ended("a blonde pop singer singing to a cat", "stop"),
                description,
                4000
            )
            .expect("finished"),
            "a blonde pop singer singing to a cat"
        );
        // Case, as Azure's word is compared everywhere else.
        for filtered in ["content_filter", "CONTENT_FILTER"] {
            let error =
                finished_rewrite(&ended("a blonde pop singer", filtered), description, 4000)
                    .expect_err("filtered with words already out");
            assert!(is_refusal(&error), "{filtered}: {error:#}");
            assert!(!format!("{error:#}").contains("singer"), "{error:#}");
        }
        for unfinished in ["length", "", "tool_calls"] {
            let error =
                finished_rewrite(&ended("a blonde pop singer", unfinished), description, 4000)
                    .expect_err("not finished");
            assert!(!is_refusal(&error), "{unfinished:?}: {error:#}");
            let said = format!("{error:#}");
            assert!(!said.contains("singer"), "{said}");
            assert!(said.contains("finish_reason"), "{said}");
        }
        // A finished rewrite is still held to the bounds after that.
        assert!(finished_rewrite(&ended("   ", "stop"), description, 4000).is_err());
    }

    /// Two requests' tokens, added — and a provider reporting nonsense
    /// saturates rather than panicking a reply it already paid for.
    #[test]
    fn usage_adds_and_saturates() {
        let a = Usage {
            prompt_tokens: 40,
            completion_tokens: 9,
        };
        let b = Usage {
            prompt_tokens: 12,
            completion_tokens: 3,
        };
        let sum = a.plus(b);
        assert_eq!((sum.prompt_tokens, sum.completion_tokens), (52, 12));
        let huge = Usage {
            prompt_tokens: i32::MAX,
            completion_tokens: i32::MAX,
        };
        let sum = huge.plus(a);
        assert_eq!(
            (sum.prompt_tokens, sum.completion_tokens),
            (i32::MAX, i32::MAX)
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
        // Still not enough, and this one is a privacy rule rather than a
        // configuration one: an assistant nobody can NAME is one no client
        // may ask permission for, so it does not exist (protocol.md,
        // "Consenting to the assistant").
        assert!(
            !cfg.is_usable(),
            "a deployment with no processor named must behave as off"
        );
        // …and THAT state is the one an upgrade lands in, so it has its own
        // question: everything filled in but the name. `main` warns on it at
        // boot, because a server that quietly lost its assistant looks
        // exactly like one whose provider is down.
        assert!(
            cfg.configured_but_nameless(),
            "an upgraded config is nameless, not half-filled"
        );
        cfg.processor = "Microsoft — Azure OpenAI".to_string();
        assert!(cfg.is_usable());
        assert!(
            !cfg.configured_but_nameless(),
            "named, so nothing to warn about"
        );
        cfg.enabled = false;
        assert!(!cfg.is_usable());
        assert!(
            !cfg.configured_but_nameless(),
            "a section switched off is not a warning either"
        );
    }

    // -- refusals ------------------------------------------------------------
    //
    // The bodies below are the shapes the providers actually send, pretty-
    // printed where they arrive pretty-printed — which is also what the
    // one-line fold exists for.

    use reqwest::StatusCode;

    /// Azure's images endpoint, refusing a description. This is the one seen
    /// in production.
    #[test]
    fn an_images_content_safety_violation_is_a_refusal() {
        let body = r#"{
  "error": {
    "code": "content_safety_violation",
    "message": "This request has been blocked by our content filters.",
    "type": null,
    "param": null
  }
}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, body));
    }

    /// Azure's `gpt-image-*` models: `contentFilter`, camel case, with the
    /// error object beside `created` rather than alone (the body Microsoft
    /// Learn documents for a refused prompt). Without its own spelling in
    /// the list it read as "some other error" and was never reworded.
    #[test]
    fn a_gpt_image_content_filter_is_a_refusal() {
        let body = r#"{"created": 1698435368, "error": {"code": "contentFilter",
            "message": "Your task failed as a result of our safety system."}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, body));
    }

    /// The classic DALL·E surface: `content_policy_violation`, with the RAI
    /// inner error underneath.
    #[test]
    fn an_images_content_policy_violation_is_a_refusal() {
        let body = r#"{"error": {"code": "content_policy_violation",
            "message": "Your request was rejected as a result of our safety system.",
            "innererror": {"code": "ResponsibleAIPolicyViolation",
                           "content_filter_results": {"violence": {"filtered": true, "severity": "medium"}}}}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, body));
        // OpenAI's image models name it differently, in the same place.
        let openai = r#"{"error": {"message": "Your request was rejected by the safety system.",
            "type": "image_generation_user_error", "param": null, "code": "moderation_blocked"}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, openai));
    }

    /// Azure's chat completions, refusing the QUESTION: `content_filter` and
    /// the RAI inner code, the way it answers a filtered prompt.
    #[test]
    fn a_chat_completions_content_filter_is_a_refusal() {
        let body = r#"{"error": {"message": "The response was filtered due to the prompt triggering Azure OpenAI's content management policy. Please modify your prompt and retry.",
            "type": null, "param": "prompt", "code": "content_filter", "status": 400,
            "innererror": {"code": "ResponsibleAIPolicyViolation",
                           "content_filter_result": {"hate": {"filtered": true, "severity": "high"}}}}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, body));
        // The inner code alone decides it, under whatever outer code.
        let inner_only = r#"{"error": {"code": "BadRequest", "message": "blocked",
            "innererror": {"code": "ResponsibleAIPolicyViolation"}}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, inner_only));
    }

    /// The message counts only under a generic or absent code — and then
    /// only a phrase naming the policy outright.
    #[test]
    fn the_message_counts_only_when_the_code_says_nothing() {
        let generic = r#"{"error": {"code": "BadRequest", "message": "The request was blocked by the RAI policy of this deployment."}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, generic));
        let absent = r#"{"error": {"message": "Blocked: ResponsibleAIPolicyViolation"}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, absent));
        // A code that names some other problem IS that problem, even if the
        // prose happens to mention the policy.
        let named = r#"{"error": {"code": "DeploymentNotFound", "message": "No RAI policy is attached to this deployment."}}"#;
        assert!(!refused_by_provider(StatusCode::NOT_FOUND, named));
        // And a generic code with ordinary prose is ordinary.
        let plain = r#"{"error": {"code": "BadRequest", "message": "Invalid value for 'size'."}}"#;
        assert!(!refused_by_provider(StatusCode::BAD_REQUEST, plain));
    }

    /// The 400s that are NOT a refusal, which are the ones an operator has
    /// to fix and a member must not be told to rephrase around.
    #[test]
    fn an_ordinary_bad_request_is_not_a_refusal() {
        let max_tokens = r#"{"error": {"message": "max_tokens is too large: 100000. This model supports at most 16384 completion tokens, whereas you provided 100000.",
            "type": "invalid_request_error", "param": "max_tokens", "code": null}}"#;
        assert!(!refused_by_provider(StatusCode::BAD_REQUEST, max_tokens));
        let unsupported = r#"{"error": {"message": "Unrecognized request argument supplied: tools",
            "type": "invalid_request_error", "param": null, "code": "unsupported_parameter"}}"#;
        assert!(!refused_by_provider(StatusCode::BAD_REQUEST, unsupported));
        let not_found = r#"{"error": {"code": "DeploymentNotFound", "message": "The API deployment for this resource does not exist."}}"#;
        assert!(!refused_by_provider(StatusCode::NOT_FOUND, not_found));
        // Not JSON at all: nothing structured to decide from.
        assert!(!refused_by_provider(StatusCode::BAD_REQUEST, "Bad Request"));
        assert!(!refused_by_provider(StatusCode::BAD_REQUEST, ""));
    }

    /// A 5xx is never a refusal, whatever its body claims: the provider did
    /// not decline the request, it failed to answer it.
    #[test]
    fn a_server_error_is_never_a_refusal() {
        let body = r#"{"error": {"code": "content_filter", "message": "The content filter is unavailable."}}"#;
        assert!(refused_by_provider(StatusCode::BAD_REQUEST, body));
        assert!(!refused_by_provider(
            StatusCode::INTERNAL_SERVER_ERROR,
            body
        ));
        assert!(!refused_by_provider(StatusCode::SERVICE_UNAVAILABLE, body));
    }

    /// A streamed answer the filter stopped, by Azure's own word for it.
    #[test]
    fn a_content_filter_finish_is_a_refusal_and_no_other_finish_is() {
        assert!(finish_is_refusal("content_filter"));
        for other in ["stop", "length", "tool_calls", ""] {
            assert!(!finish_is_refusal(other), "{other:?}");
        }
    }

    /// The decision survives the context the log line is written in, and
    /// any context a caller adds on top — while an ordinary failure, with
    /// the same kind of line, is not mistaken for one.
    #[test]
    fn a_refusal_is_found_through_the_error_chain() {
        let refused = provider_error(
            StatusCode::BAD_REQUEST,
            "image generation returned 400".to_string(),
            r#"{"error": {"code": "content_safety_violation"}}"#,
        );
        assert!(is_refusal(&refused));
        assert!(is_refusal(&refused.context("drawing the picture")));
        let ordinary = provider_error(
            StatusCode::BAD_REQUEST,
            "image generation returned 400".to_string(),
            r#"{"error": {"code": "invalid_size"}}"#,
        );
        assert!(!is_refusal(&ordinary));
        // The log line still says what happened, and then why.
        let line = format!(
            "{:#}",
            provider_error(
                StatusCode::BAD_REQUEST,
                "assistant returned 400".to_string(),
                r#"{"error": {"code": "content_filter"}}"#,
            )
        );
        assert_eq!(
            line,
            "assistant returned 400: the provider's content filter refused it"
        );
    }

    /// The provider's detail reaches the log on ONE line, bounded, and never
    /// cut mid-letter.
    #[test]
    fn the_provider_detail_is_folded_onto_one_line() {
        let pretty = "{\n  \"error\": {\n    \"code\": \"content_filter\",\r\n\t\"message\": \"x\"\n  }\n}\n";
        assert_eq!(
            one_line(pretty),
            r#"{ "error": { "code": "content_filter", "message": "x" } }"#
        );
        let long = format!("{{\n{}\n}}", "я".repeat(1000));
        let folded = one_line(&long);
        assert_eq!(folded.chars().count(), 400);
        assert!(!folded.contains('\n'));
    }

    /// A refused picture: the log names the refusal and the category that
    /// tripped, and never the description — not the `revised_prompt` a
    /// DALL·E 3 refusal can carry, not a message that quotes it.
    #[test]
    fn a_refused_description_never_reaches_the_log() {
        let body = r#"{
          "error": {
            "code": "contentFilter",
            "message": "Your task failed as a result of our safety system: 'a purple giraffe'",
            "inner_error": {
              "code": "ResponsibleAIPolicyViolation",
              "content_filter_results": {
                "hate": {"filtered": false, "severity": "safe"},
                "violence": {"filtered": true, "severity": "medium"},
                "jailbreak": {"filtered": true, "detected": true}
              },
              "revised_prompt": "A purple giraffe wearing a hat, in watercolour"
            }
          }
        }"#;
        let line = format!(
            "{:#}",
            provider_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "image generation returned 400 for https://example.test/images: {}",
                    loggable_detail(body)
                ),
                body,
            )
        );
        assert!(!line.to_lowercase().contains("giraffe"), "{line}");
        assert!(!line.contains("safety system"), "{line}");
        assert!(line.contains("code=contentFilter"), "{line}");
        assert!(
            line.contains("inner=ResponsibleAIPolicyViolation"),
            "{line}"
        );
        assert!(
            line.contains("filtered=jailbreak,violence:medium"),
            "{line}"
        );
        assert!(!line.contains("hate"), "{line}");
        assert!(!line.contains('\n'));
        assert!(
            line.ends_with("the provider's content filter refused it"),
            "{line}"
        );
    }

    /// A 422 that echoes the request under `detail[].input` keeps the field
    /// and the rule, and drops the echo.
    #[test]
    fn a_validation_echo_never_reaches_the_log() {
        let body = r#"{"detail": [
            {"type": "string_too_long", "loc": ["body", "prompt"],
             "msg": "String should have at most 4000 characters",
             "input": "Grandma's secret birthday cake with seven candles"},
            {"type": "extra_forbidden", "loc": ["body", "size"], "input": "1024x1024"}
        ]}"#;
        let line = loggable_detail(body);
        assert!(!line.contains("Grandma"), "{line}");
        assert!(!line.contains("1024x1024"), "{line}");
        assert!(
            line.starts_with("invalid=body.prompt:string_too_long,body.size:extra_forbidden "),
            "{line}"
        );
    }

    /// The chat completions' shape: the identifiers are kept, the message is
    /// not — an ordinary 400's message can quote the value it rejected.
    #[test]
    fn only_identifiers_are_kept_from_an_ordinary_error() {
        let body = r#"{"error": {"message": "Invalid value: 'tell me about Aunt Vera'",
            "type": "invalid_request_error", "param": "messages[1].content", "code": null}}"#;
        let line = loggable_detail(body);
        assert!(!line.contains("Vera"), "{line}");
        assert_eq!(
            line,
            format!(
                "type=invalid_request_error param=messages[1].content {}-byte body, other fields withheld",
                body.len()
            )
        );
        // A "code" that is prose is prose, whichever field it sits in.
        let prose = r#"{"error": {"code": "draw a cat for Vera", "message": "x"}}"#;
        assert!(!loggable_detail(prose).contains("Vera"));
        // Azure's 404 keeps its numeric code.
        assert!(
            loggable_detail(r#"{"error": {"code": "404", "message": "Resource not found"}}"#)
                .starts_with("code=404 ")
        );
        // Not JSON: nothing structured to keep, so nothing but its size.
        assert_eq!(
            loggable_detail("<html>bad gateway for 'a cat'</html>"),
            "36-byte body, not JSON, withheld"
        );
        assert_eq!(loggable_detail(""), "0-byte body, not JSON, withheld");
    }

    /// However many categories a body lists, the line stays one line within
    /// the 400-character bound.
    #[test]
    fn the_loggable_detail_keeps_the_bound() {
        let categories: Vec<String> = (0..200)
            .map(|n| format!(r#""category_{n}": {{"filtered": true, "severity": "high"}}"#))
            .collect();
        let body = format!(
            r#"{{"error": {{"code": "content_filter", "innererror": {{"content_filter_result": {{{}}}}}}}}}"#,
            categories.join(",")
        );
        let line = loggable_detail(&body);
        assert!(line.chars().count() <= 400);
        assert!(
            line.starts_with("code=content_filter filtered=category_"),
            "{line}"
        );
    }

    #[test]
    fn the_transcription_hint_is_the_bare_language() {
        use super::transcription_language;
        assert_eq!(transcription_language("ru").as_deref(), Some("ru"));
        assert_eq!(transcription_language("sr-Latn").as_deref(), Some("sr"));
        assert_eq!(transcription_language("sr").as_deref(), Some("sr"));
        assert_eq!(transcription_language("zh-Hans").as_deref(), Some("zh"));
        assert_eq!(transcription_language(" EN ").as_deref(), Some("en"));
        assert_eq!(transcription_language(""), None);
        assert_eq!(transcription_language("e"), None);
        assert_eq!(transcription_language("english"), None);
        assert_eq!(transcription_language("1a"), None);
    }

    #[test]
    fn a_transcript_answer_is_read_for_its_text_and_a_shaped_language() {
        use super::{Transcript, parse_transcript};
        assert_eq!(
            parse_transcript(br#"{"text": " hello there "}"#).expect("parses"),
            Transcript {
                text: "hello there".to_string(),
                language: None
            }
        );
        assert_eq!(
            parse_transcript(br#"{"text": "", "language": "russian"}"#).expect("silence"),
            Transcript {
                text: String::new(),
                language: Some("russian".to_string())
            },
            "silence is an answer"
        );
        let prose = parse_transcript(br#"{"text": "x", "language": "not a language at all!"}"#)
            .expect("parses");
        assert_eq!(prose.language, None, "prose is not passed on");
        assert!(
            parse_transcript(br#"{"words": "x"}"#).is_err(),
            "no text is a failure"
        );
        assert!(parse_transcript(b"not json").is_err());
    }

    // --- what leaves on the wire ------------------------------------------

    /// Serve ONE HTTP request on a local port, answer it with `status` and
    /// `answer`, and hand back the raw bytes of what arrived — the request
    /// line, every header and the whole body, exactly as this file sent
    /// them. Nothing here reaches a real provider.
    async fn capture_one(
        status: &'static str,
        answer: impl Into<String>,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let answer: String = answer.into();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding a capture port");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("one request");
            let mut raw: Vec<u8> = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = socket.read(&mut buf).await.expect("reading the request");
                assert!(n > 0, "the request ended early");
                raw.extend_from_slice(&buf[..n]);
                let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map(|value| value.trim().parse::<usize>().expect("a length"));
                let done = match length {
                    Some(length) => raw.len() >= end + 4 + length,
                    None => raw.ends_with(b"0\r\n\r\n"),
                };
                if done {
                    break;
                }
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{answer}",
                answer.len()
            );
            socket.write_all(reply.as_bytes()).await.expect("answering");
            raw
        });
        (base, task)
    }

    /// The raw request with the two things that differ per run — the
    /// multipart boundary and the capture port — replaced by fixed words.
    fn normalised(raw: &[u8], base: &str) -> String {
        let text = String::from_utf8_lossy(raw).into_owned();
        let boundary = text
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .starts_with("content-type: multipart/form-data")
                    .then(|| line.split("boundary=").nth(1).map(str::to_string))
                    .flatten()
            })
            .expect("a multipart boundary");
        let host = base.trim_start_matches("http://");
        text.replace(&boundary, "BOUNDARY").replace(host, "HOST")
    }

    /// The OpenAI transcription request, BYTE FOR BYTE — written against
    /// the server as it was before a second contract existed, and held
    /// here so `api = "openai"` (the default) keeps sending exactly it.
    #[tokio::test]
    async fn the_openai_transcription_request_is_byte_for_byte_what_it_was() {
        let (base, captured) = capture_one("200 OK", r#"{"text": "hi", "language": "ru"}"#).await;
        let route = TranscribeRoute {
            route: ModelRoute {
                url: format!(
                    "{base}/openai/deployments/w/audio/transcriptions?api-version=2024-10-21"
                ),
                api_key: "the-key".to_string(),
                auth: AuthScheme::ApiKey,
                model: "w".to_string(),
                max_tokens: 1024,
            },
            contract: TranscribeContract::OpenAi,
            // The default for this contract — what it always did.
            language_hint: true,
        };
        let recording = Recording {
            bytes: b"\x00\x00\x00\x18ftypM4A sound".to_vec(),
            mime: "audio/mp4",
            filename: "audio.m4a",
        };
        let client = reqwest::Client::new();
        let answered = transcribe(&client, &route, recording, Some("ru"))
            .await
            .expect("answered");
        assert_eq!(answered.transcript.text, "hi");
        let raw = captured.await.expect("captured");
        let got = normalised(&raw, &base);
        assert_eq!(
            got,
            concat!(
                "POST /openai/deployments/w/audio/transcriptions?api-version=2024-10-21 HTTP/1.1\r\n",
                "api-key: the-key\r\n",
                "content-type: multipart/form-data; boundary=BOUNDARY\r\n",
                "content-length: 640\r\n",
                "accept: */*\r\n",
                "host: HOST\r\n",
                "\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"file\"; filename=\"audio.m4a\"\r\n",
                "Content-Type: audio/mp4\r\n",
                "\r\n",
                "\0\0\0\u{18}ftypM4A sound\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"model\"\r\n",
                "\r\n",
                "w\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"response_format\"\r\n",
                "\r\n",
                "json\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"language\"\r\n",
                "\r\n",
                "ru\r\n",
                "--BOUNDARY--\r\n",
            )
        );
    }

    fn speech_route(base: &str, hint: bool) -> TranscribeRoute {
        TranscribeRoute {
            route: ModelRoute {
                url: format!(
                    "{base}/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
                ),
                api_key: "speech-key".to_string(),
                auth: AuthScheme::SubscriptionKey,
                model: "MAI-Transcribe-2".to_string(),
                max_tokens: 1024,
            },
            contract: TranscribeContract::Speech {
                style: crate::config::TranscribeStyle::Clean,
            },
            language_hint: hint,
        }
    }

    fn voice() -> Recording {
        Recording {
            bytes: b"\x00\x00\x00\x18ftypM4A voice".to_vec(),
            mime: "audio/mp4",
            filename: "audio.m4a",
        }
    }

    const SPEECH_ANSWER: &str = r#"{"durationMilliseconds": 5300,
        "combinedPhrases": [{"text": "We will be there at six."}],
        "phrases": [{"text": "We will be there at six.", "locale": "en-US",
                     "offsetMilliseconds": 0, "durationMilliseconds": 5300, "confidence": 0.9}]}"#;

    /// THE SPEECH REQUEST, byte for byte: the key in the subscription
    /// header and in no other, and two parts — the sound as `audio` under
    /// its stored name and type, and the `definition` naming the model and
    /// the style. No locale, though the family has a language: the hint is
    /// off by default for this contract.
    #[tokio::test]
    async fn the_speech_request_is_the_sound_and_a_definition_and_nothing_else() {
        let (base, captured) = capture_one("200 OK", SPEECH_ANSWER).await;
        let client = reqwest::Client::new();
        let answered = transcribe(&client, &speech_route(&base, false), voice(), Some("ru"))
            .await
            .expect("answered");
        assert_eq!(
            answered,
            Transcription {
                transcript: Transcript {
                    text: "We will be there at six.".to_string(),
                    language: Some("en-US".to_string()),
                },
                audio_ms: Some(5300),
            }
        );
        let raw = captured.await.expect("captured");
        let got = normalised(&raw, &base);
        let head = got
            .split("\r\n\r\n")
            .next()
            .expect("head")
            .to_ascii_lowercase();
        assert!(!head.contains("api-key"), "{got:?}");
        assert!(!head.contains("authorization"), "{got:?}");
        let request_line = head.lines().next().unwrap_or_default();
        assert!(
            !request_line.contains("speech-key"),
            "never in the URL: {got:?}"
        );
        assert_eq!(
            got,
            concat!(
                "POST /speechtotext/transcriptions:transcribe?api-version=2025-10-15 HTTP/1.1\r\n",
                "ocp-apim-subscription-key: speech-key\r\n",
                "content-type: multipart/form-data; boundary=BOUNDARY\r\n",
                "content-length: 487\r\n",
                "accept: */*\r\n",
                "host: HOST\r\n",
                "\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"audio\"; filename=\"audio.m4a\"\r\n",
                "Content-Type: audio/mp4\r\n",
                "\r\n",
                "\0\0\0\u{18}ftypM4A voice\r\n",
                "--BOUNDARY\r\n",
                "Content-Disposition: form-data; name=\"definition\"\r\n",
                "\r\n",
                "{\"enhancedMode\":{\"enabled\":true,\"model\":\"MAI-Transcribe-2\",",
                "\"modelOptions\":{\"transcribeStyle\":\"clean\"}}}\r\n",
                "--BOUNDARY--\r\n",
            )
        );
    }

    /// The definition, pinned: model, style, and ONE locale only when one
    /// is given.
    #[test]
    fn the_speech_definition_carries_the_model_the_style_and_at_most_one_locale() {
        assert_eq!(
            speech_definition("MAI-Transcribe-2", "clean", None).to_string(),
            r#"{"enhancedMode":{"enabled":true,"model":"MAI-Transcribe-2","modelOptions":{"transcribeStyle":"clean"}}}"#
        );
        assert_eq!(
            speech_definition("MAI-Transcribe-1.5", "verbatim", Some("ru")),
            json!({"enhancedMode": {"enabled": true, "model": "MAI-Transcribe-1.5",
                                    "modelOptions": {"transcribeStyle": "verbatim"}},
                   "locales": ["ru"]})
        );
    }

    /// The hint: off by default for speech, on when opted in — and never
    /// Serbian, in either alphabet, whatever the operator chose. The OpenAI
    /// contract sends what it always sent, unless turned off.
    #[test]
    fn the_speech_contract_sends_a_locale_only_when_opted_in_and_never_serbian() {
        let off = speech_route("http://x", false);
        let on = speech_route("http://x", true);
        assert_eq!(hint_for(&off, Some("ru")), None);
        assert_eq!(hint_for(&on, Some("ru")).as_deref(), Some("ru"));
        assert_eq!(hint_for(&on, Some("zh")).as_deref(), Some("zh"));
        assert_eq!(hint_for(&on, None), None);
        for serbian in ["sr", "SR", "sr-Latn"] {
            assert_eq!(hint_for(&on, Some(serbian)), None, "{serbian}");
        }
        // What the handler hands over for both Serbian choices is `sr`.
        assert_eq!(
            hint_for(&on, transcription_language("sr-Latn").as_deref()),
            None
        );
        let mut openai = on.clone();
        openai.contract = TranscribeContract::OpenAi;
        assert_eq!(hint_for(&openai, Some("sr")).as_deref(), Some("sr"));
        openai.language_hint = false;
        assert_eq!(hint_for(&openai, Some("sr")), None);
    }

    /// With the hint on, the locale is IN the request — and a Serbian
    /// family's is not.
    #[tokio::test]
    async fn an_opted_in_locale_reaches_the_definition_and_serbian_never_does() {
        for (family, expected) in [(Some("de"), Some("de")), (Some("sr"), None)] {
            let (base, captured) = capture_one("200 OK", SPEECH_ANSWER).await;
            let client = reqwest::Client::new();
            transcribe(&client, &speech_route(&base, true), voice(), family)
                .await
                .expect("answered");
            let got = normalised(&captured.await.expect("captured"), &base);
            let definition = got
                .split("name=\"definition\"\r\n\r\n")
                .nth(1)
                .and_then(|rest| rest.split("\r\n").next())
                .expect("a definition part");
            let definition: Value = serde_json::from_str(definition).expect("JSON");
            match expected {
                Some(locale) => assert_eq!(definition["locales"], json!([locale]), "{family:?}"),
                None => assert!(
                    definition.get("locales").is_none(),
                    "{family:?}: {definition}"
                ),
            }
        }
    }

    /// The answer: channels in order, silence as `""`, the first phrase's
    /// locale, the provider's duration — and a 200 of some other contract is
    /// a failure, not silence.
    #[test]
    fn a_speech_answer_is_read_in_channel_order_with_its_duration() {
        let (transcript, ms) = parse_speech_transcript(
            br#"{"durationMilliseconds": 9000,
                 "combinedPhrases": [{"channel": 1, "text": "second"},
                                     {"channel": 0, "text": " first "},
                                     {"channel": 1, "text": "third"},
                                     {"channel": 0, "text": ""}],
                 "phrases": [{"text": "first", "locale": "ru-RU"},
                             {"text": "second", "locale": "en-US"}]}"#,
        )
        .expect("parses");
        assert_eq!(transcript.text, "first\nsecond\nthird");
        assert_eq!(transcript.language.as_deref(), Some("ru-RU"));
        assert_eq!(ms, Some(9000));

        // Empty lists: silence.
        let (silent, ms) = parse_speech_transcript(
            br#"{"durationMilliseconds": 1200, "combinedPhrases": [], "phrases": []}"#,
        )
        .expect("silence");
        assert_eq!(
            silent,
            Transcript {
                text: String::new(),
                language: None
            }
        );
        assert_eq!(ms, Some(1200));
        let (silent, ms) =
            parse_speech_transcript(br#"{"combinedPhrases": [], "phrases": []}"#).expect("silence");
        assert_eq!(silent.text, "");
        assert_eq!(ms, None, "no duration given");

        // A locale that is prose is not handed on; a negative duration is not one.
        let (odd, ms) = parse_speech_transcript(
            br#"{"durationMilliseconds": -5, "combinedPhrases": [{"text": "x"}],
                 "phrases": [{"text": "x", "locale": "not a locale!"}]}"#,
        )
        .expect("parses");
        assert_eq!(odd.language, None);
        assert_eq!(ms, None);

        assert!(parse_speech_transcript(br#"{"text": "openai shape"}"#).is_err());
        assert!(parse_speech_transcript(b"{}").is_err());
        assert!(parse_speech_transcript(b"not json").is_err());
    }

    /// What was said is never read as silence: with `combinedPhrases`
    /// missing, not a list, or holding no text, the text is the phrases'
    /// own — a space within a channel, a line between channels. A 200 with
    /// NEITHER list (only a duration) is no answer at all.
    #[test]
    fn a_speech_answer_without_combined_text_is_read_from_its_phrases() {
        let phrases = r#""phrases": [{"text": "We will be", "locale": "en-US"},
                                    {"channel": 1, "text": "Fine."},
                                    {"text": " there at six. "},
                                    {"text": ""}]"#;
        for combined in [
            "",
            r#""combinedPhrases": "x","#,
            r#""combinedPhrases": [{"text": " "}],"#,
        ] {
            let body = format!(r#"{{"durationMilliseconds": 10, {combined} {phrases}}}"#);
            let (transcript, ms) = parse_speech_transcript(body.as_bytes()).expect("parses");
            assert_eq!(transcript.text, "We will be there at six.\nFine.", "{body}");
            assert_eq!(transcript.language.as_deref(), Some("en-US"));
            assert_eq!(ms, Some(10));
        }
        // combinedPhrases wins when it has text.
        let (transcript, _) = parse_speech_transcript(
            br#"{"combinedPhrases": [{"text": "combined"}], "phrases": [{"text": "phrase"}]}"#,
        )
        .expect("parses");
        assert_eq!(transcript.text, "combined");

        for bare in [
            r#"{"durationMilliseconds": 1200}"#,
            r#"{"durationMilliseconds": 1200, "combinedPhrases": null}"#,
            r#"{"combinedPhrases": {"text": "x"}, "phrases": "x"}"#,
        ] {
            assert!(parse_speech_transcript(bare.as_bytes()).is_err(), "{bare}");
        }
    }

    fn speech_error(code: &str, inner: &str) -> String {
        json!({"code": code, "message": format!("about {inner}: We will be there at six"),
               "innerError": {"code": inner, "message": "We will be there at six"}})
        .to_string()
    }

    /// The codes decide, never the message: a refused FILE — its format, its
    /// length, or no audio found in it — is unreadable, an unidentified
    /// language is unheard, neither is ever silence, and everything else —
    /// a 429, a 5xx, a bad locale — is a transient failure. Nothing here is
    /// ever a content refusal.
    #[test]
    fn a_speech_refusal_is_mapped_by_its_codes() {
        use reqwest::StatusCode as S;
        let cases = [
            (
                S::BAD_REQUEST,
                speech_error("InvalidRequest", "InvalidAudioFormat"),
                SpeechFailure::Unreadable,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidRequest", "AudioLengthLimitExceeded"),
                SpeechFailure::Unreadable,
            ),
            (
                S::BAD_REQUEST,
                speech_error("UnsupportedMediaType", "Unknown"),
                SpeechFailure::Unreadable,
            ),
            (
                S::UNSUPPORTED_MEDIA_TYPE,
                String::new(),
                SpeechFailure::Unreadable,
            ),
            (
                S::PAYLOAD_TOO_LARGE,
                String::new(),
                SpeechFailure::Unreadable,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidRequest", "EmptyAudioFile"),
                SpeechFailure::Unreadable,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidRequest", "NoLanguageIdentified"),
                SpeechFailure::Unheard,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidArgument", "UnsupportedLanguageCode"),
                SpeechFailure::Failed,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidArgument", "InvalidLocale"),
                SpeechFailure::Failed,
            ),
            (
                S::BAD_REQUEST,
                speech_error("InvalidRequest", "MultipleLanguagesIdentified"),
                SpeechFailure::Failed,
            ),
            (
                S::TOO_MANY_REQUESTS,
                speech_error("TooManyRequests", "TooManyRequests"),
                SpeechFailure::Failed,
            ),
            (
                S::INTERNAL_SERVER_ERROR,
                speech_error("InternalServerError", "InvalidAudioFormat"),
                SpeechFailure::Failed,
            ),
            (S::SERVICE_UNAVAILABLE, String::new(), SpeechFailure::Failed),
            // A message naming a code decides nothing.
            (
                S::BAD_REQUEST,
                json!({"code": "InvalidRequest", "message": "InvalidAudioFormat"}).to_string(),
                SpeechFailure::Failed,
            ),
            (
                S::BAD_REQUEST,
                "not json".to_string(),
                SpeechFailure::Failed,
            ),
        ];
        for (status, body, expected) in cases {
            assert_eq!(speech_failure(status, &body), expected, "{status} {body}");
        }
    }

    /// End to end through `transcribe`: a refused format, and an empty
    /// file, are [`Unreadable`] in the chain and never [`Refused`]; an
    /// unidentified language is [`Unheard`]; none of them is an answer of
    /// `""`; a 429 is a plain failure. The error carries the codes and not
    /// the message.
    #[tokio::test]
    async fn a_speech_refusal_reaches_the_caller_as_unreadable_unheard_or_failure() {
        let client = reqwest::Client::new();

        let body = speech_error("InvalidRequest", "InvalidAudioFormat");
        let (base, captured) = capture_one("400 Bad Request", body).await;
        let err = transcribe(&client, &speech_route(&base, false), voice(), None)
            .await
            .unwrap_err();
        captured.await.expect("captured");
        assert!(is_unreadable(&err), "{err:#}");
        assert!(!is_refusal(&err), "{err:#}");
        let text = format!("{err:#}");
        assert!(text.contains("inner=InvalidAudioFormat"), "{text}");
        assert!(!text.contains("six"), "the message is withheld: {text}");

        for (inner, unreadable, unheard) in [
            ("EmptyAudioFile", true, false),
            ("NoLanguageIdentified", false, true),
        ] {
            let body = speech_error("InvalidRequest", inner);
            let (base, captured) = capture_one("400 Bad Request", body).await;
            let err = transcribe(&client, &speech_route(&base, false), voice(), None)
                .await
                .expect_err("a refusal is never an answer of silence");
            captured.await.expect("captured");
            assert_eq!(is_unreadable(&err), unreadable, "{inner}: {err:#}");
            assert_eq!(is_unheard(&err), unheard, "{inner}: {err:#}");
            assert!(!is_refusal(&err), "{err:#}");
            assert!(!format!("{err:#}").contains("six"), "{err:#}");
        }

        let body = speech_error("TooManyRequests", "TooManyRequests");
        let (base, captured) = capture_one("429 Too Many Requests", body).await;
        let err = transcribe(&client, &speech_route(&base, false), voice(), None)
            .await
            .unwrap_err();
        captured.await.expect("captured");
        assert!(
            !is_unreadable(&err) && !is_unheard(&err) && !is_refusal(&err),
            "{err:#}"
        );
    }

    /// The speech contract spells its inner error `innerError`; the log's
    /// allow-list reads it, and still keeps no message.
    #[test]
    fn the_log_reads_the_speech_contracts_inner_code_and_withholds_its_message() {
        let detail = loggable_detail(&speech_error("InvalidRequest", "NoLanguageIdentified"));
        assert!(detail.contains("code=InvalidRequest"), "{detail}");
        assert!(detail.contains("inner=NoLanguageIdentified"), "{detail}");
        assert!(!detail.contains("six"), "{detail}");
    }
}
