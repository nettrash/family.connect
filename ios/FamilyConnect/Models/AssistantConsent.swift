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
    ///
    /// `transcribes` adds the line protocol.md requires once a server can
    /// turn a recording into text: asking for one sends that recording's
    /// SOUND to the processor — a new kind of thing leaving the server, so
    /// it is said in a line of its own (protocol.md, "Consenting to the
    /// assistant", amended 2026-10-02).
    static func disclosure(
        processor: String,
        familyHistory: Bool,
        familyVision: Bool,
        transcribes: Bool = false
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
        if transcribes {
            lines.append(String(localized: "If you ask for the text of a voice note, audio file or video, its sound is sent to \(processor)."))
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

// MARK: - Looking things up

/// The second consent (protocol.md, "Consenting to the assistant", amended
/// 2026-10-03, and "Looking things up"): whether the assistant may send a
/// short query or place name it wrote from this member's question to the
/// providers `assistant.lookups` names — a party that is NOT `processor`,
/// so the first consent is not stretched to cover it.
///
/// Unlike the first, nothing is ever REFUSED for the want of it: a member
/// who never agrees keeps the assistant exactly as it was. So it is never
/// asked for on the way to sending a message on its own — it is offered on
/// the same screen as the first consent, as a second way to agree, and in
/// Settings afterwards.
nonisolated extension AssistantConsent {

    /// What a member chose on the consent screen.
    enum Answer: Equatable, Sendable {
        /// "I Agree" / "Agree Without Lookups": the assistant only.
        case assistant
        /// "Agree With Lookups": both, in that order.
        case assistantAndLookups
        /// "I Agree" on the lookup screen of somebody who already agreed to
        /// the assistant: the second consent alone.
        case lookups
    }

    /// One server write.
    enum Step: Equatable, Sendable {
        /// `POST /me/assistant-consent {"granted": true}`.
        case assistant
        /// `POST /me/assistant-lookup-consent {"granted": true}`.
        case lookups
    }

    /// The writes an answer makes, in the order they must be made: the
    /// server refuses to grant the lookup consent without the assistant
    /// consent (`assistant_consent_required`), so the assistant's always
    /// comes first — and never twice.
    static func steps(for answer: Answer) -> [Step] {
        switch answer {
        case .assistant: [.assistant]
        case .assistantAndLookups: [.assistant, .lookups]
        case .lookups: [.lookups]
        }
    }

    /// What the consent screen asks.
    enum SheetMode: Equatable, Sendable {
        /// The assistant consent alone, as before lookups existed: the
        /// server has no lookup source, so there is nothing more to ask.
        case assistant
        /// Both on one screen: the assistant's lines, then the lookup line
        /// naming `providers`, and two ways to agree.
        case assistantWithLookups(providers: String)
        /// The member agreed to the assistant already; only the lookup
        /// line, and one way to agree.
        case lookupsOnly(providers: String)
    }

    static func sheetMode(assistantAgreed: Bool, lookups: [String]?) -> SheetMode {
        guard let providers = providerList(lookups ?? []) else { return .assistant }
        return assistantAgreed ? .lookupsOnly(providers: providers) : .assistantWithLookups(providers: providers)
    }

    /// The provider names as one phrase for a sentence — "Brave Search,
    /// Open-Meteo and Wikipedia" — or nil when there is nobody to name, in
    /// which case nothing about lookups is said or asked at all: a client
    /// that cannot name the providers does not ask (protocol.md).
    ///
    /// Two and three names go through the catalogue, because "and" and the
    /// comma are the translator's (Japanese and Chinese join with 、 and
    /// と / 和). The server sends at most three; a longer list from a
    /// future server falls back to the system's own list formatter rather
    /// than dropping names somebody has to be told about.
    static func providerList(_ names: [String]) -> String? {
        let names = names
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        switch names.count {
        case 0:
            return nil
        case 1:
            return names[0]
        case 2:
            return String(localized: "\(names[0]) and \(names[1])")
        case 3:
            return String(localized: "\(names[0]), \(names[1]) and \(names[2])")
        default:
            return ListFormatter.localizedString(byJoining: names)
        }
    }

    /// May this client offer lookups at all — the switch, the consent line,
    /// the Settings section? Only where it may offer the assistant (a named
    /// processor) AND the server named at least one provider.
    static func offersLookups(processor: String?, lookups: [String]?) -> Bool {
        isAvailable(processor: processor) && providerList(lookups ?? []) != nil
    }

    /// The consent screen's lookup line (protocol.md, "what a client must
    /// say before it asks"). Which of the two depends on `ai_history`, the
    /// way the family-chat lines above do: with it on, the query may be
    /// shaped by the chat's recent messages too, and saying otherwise would
    /// be the lie by omission that section forbids.
    static func lookupDisclosure(providers: String, familyHistory: Bool) -> String {
        if familyHistory {
            return String(localized: "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat, possibly from recent messages too — to \(providers), and its answer then lists its sources.")
        }
        return String(localized: "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to \(providers), and its answer then lists its sources.")
    }

    /// The member's lookup section in Settings.
    enum LookupSettings: Equatable, Sendable {
        /// Not drawn: the server has no source, or no assistant this client
        /// may offer, or the member has not agreed to the assistant yet —
        /// the assistant section's own "Review and Agree…" asks both then,
        /// and a second button opening the same screen would be noise.
        case absent
        /// Offered: "Review and Allow Lookups…".
        case notAgreed(providers: String)
        /// Agreed on this date: "Stop Lookups".
        case agreed(at: Date, providers: String)
    }

    static func lookupSettings(
        processor: String?,
        lookups: [String]?,
        assistantAgreedAt: Date?,
        lookupAgreedAt: Date?
    ) -> LookupSettings {
        guard isAvailable(processor: processor),
              let providers = providerList(lookups ?? []),
              assistantAgreedAt != nil
        else { return .absent }
        if let lookupAgreedAt { return .agreed(at: lookupAgreedAt, providers: providers) }
        return .notAgreed(providers: providers)
    }
}
