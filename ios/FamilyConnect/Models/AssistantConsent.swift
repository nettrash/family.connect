//
//  AssistantConsent.swift
//  FamilyConnect
//
//  Whether this member has agreed that their words may go to the model,
//  and what they have to be told before they answer (docs/protocol.md,
//  "Consenting to the assistant").
//
//  The assistant is the only place in this product where something a
//  person writes leaves the server their family chose, so it is the only
//  place that asks. The answer is kept on the SERVER — `assistant_consent_at`
//  on `GET /me`, set by `POST /me/assistant-consent` — because the server
//  is what calls the model, a reinstall must not quietly re-ask and
//  re-send, and somebody who agreed on their phone has agreed rather than
//  agreed-on-that-phone.
//
//  Pure logic and no view: the same three questions are asked by the
//  phone's composer, the Mac's, and the settings screens of both, and one
//  of them getting a different answer is how a message gets refused after
//  it was typed.
//

import Foundation

nonisolated enum AssistantConsent {
    /// Would a message with this body, in this chat, be sent to the model?
    ///
    /// The MIRROR of `model_surface` in `server/src/handlers_chat.rs`: the
    /// member's own `ai` chat, where everything goes, and the family chat,
    /// where only an `@ai` does. `/draw` needs no case of its own — in the
    /// family chat it is `@ai /draw`, a mention, and in the assistant's own
    /// chat it is that chat. If the two ever disagree, a message is either
    /// refused after it was typed or sent having asked nothing, so they are
    /// pinned against the same vectors on both sides.
    static func reachesTheModel(chatKind: String?, body: String) -> Bool {
        switch chatKind {
        case "ai": true
        case "family": AssistantMention.mentions(body)
        default: false
        }
    }

    /// Must this member be asked before this message is sent?
    ///
    /// False when the server has no assistant — there is nothing to
    /// consent to and nothing will be called — and false once they have
    /// agreed. `processor` is part of "has an assistant" on purpose: a
    /// client that cannot NAME who receives the words cannot ask the
    /// question, so it does not offer the assistant at all.
    static func isRequired(
        chatKind: String?,
        body: String,
        processor: String?,
        agreedAt: Date?
    ) -> Bool {
        guard isAvailable(processor: processor) else { return false }
        guard agreedAt == nil else { return false }
        return reachesTheModel(chatKind: chatKind, body: body)
    }

    /// Is there an assistant this client may offer at all?
    ///
    /// A server that names no processor has one this client must not use:
    /// the disclosure has a hole in exactly the place the person needs to
    /// read, and "some third party" is not an answer somebody can weigh.
    static func isAvailable(processor: String?) -> Bool {
        guard let processor else { return false }
        return !processor.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Would this message reach a model whose owner the server will not
    /// name — so this client must hold it back entirely?
    ///
    /// The case is a server with an assistant configured that predates
    /// `processor`. Consent cannot be asked for there, because the screen
    /// would have a hole exactly where the person needs to read, and
    /// sending anyway is the thing this whole feature exists to stop. So
    /// the message does not go: the composer says so, and the assistant is
    /// simply off until that server is updated.
    ///
    /// `hasAssistant` is what keeps this from swallowing ordinary words.
    /// On a server with NO assistant, `@ai` in the family chat is three
    /// characters of text that reach nobody, and a client refusing to send
    /// them would be breaking a conversation to protect nothing.
    static func isWithheldFromAnUnnamedAssistant(
        chatKind: String?,
        body: String,
        hasAssistant: Bool,
        processor: String?
    ) -> Bool {
        guard hasAssistant, !isAvailable(processor: processor) else { return false }
        return reachesTheModel(chatKind: chatKind, body: body)
    }

    /// What the person is told BEFORE they answer, in the order they are
    /// shown. All of it on the screen where they answer, not only in a
    /// policy behind a link (App Store Review Guideline 5.1.1(i), and
    /// protocol.md, "What a client must say before it asks").
    ///
    /// The two family-chat lines depend on the owner's `ai_history`: with
    /// it on a mention takes the chat's recent history with it, and with it
    /// off it takes nothing but itself. Saying the wrong one of those would
    /// be worse than saying neither.
    static func disclosure(
        processor: String,
        familyHistory: Bool,
        familyVision: Bool
    ) -> [String] {
        // One line per `String(localized:)` call, long as they are: the
        // catalogue checker reads source text, and a call wrapped over
        // several lines is a key it cannot see — which is how a string
        // ships English in nine languages with nothing failing to say so.
        var lines = [
            String(localized: "What you write to the assistant leaves this family's server and is sent to \(processor).")
        ]
        if familyHistory {
            lines.append(String(localized: "In the family chat only a message that says \(AssistantMention.token) is sent — and with it the last 30 days of that chat, up to 200 messages, including what other people wrote, their names and the times."))
        } else {
            lines.append(String(localized: "In the family chat only a message that says \(AssistantMention.token) is sent, and nothing else from that chat goes with it."))
        }
        if familyVision {
            lines.append(String(localized: "A photo is sent only when you attach one to a message for the assistant, and only while your family allows it."))
        }
        lines.append(String(localized: "The answer comes back as a message in that chat, where everyone in the chat can read it."))
        lines.append(String(localized: "You can stop this at any time in Settings. What has already been sent cannot be taken back."))
        return lines
    }

    /// The one line a composer shows where the assistant cannot be used
    /// because the server named nobody.
    static var unnamedProcessorNotice: String {
        String(localized: "This server hasn't said which service answers, so nothing can be sent to the assistant here.")
    }
}
