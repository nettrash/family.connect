//
//  MessageIntents.swift
//  FamilyConnect
//
//  The Shortcuts surface (issue #118): "Send a Message" and "Send an
//  Attachment", each aimed at a family member or the family chat, with the
//  photos, videos, voice recordings and files a Shortcut is holding.
//
//  IN THE APP, NOT IN AN EXTENSION. These live in the app target, so the
//  system runs them by launching this app and the send happens where every
//  other send happens — with the keychain, the store and the outbox all
//  present. An App Intents extension would need the App Group and a shared
//  keychain access for the session token to answer from another process;
//  it would buy a faster launch and cost a second place the session is
//  read. The calling side already made the same choice for the same reason
//  (CallIntents: "no Intents extension, the roster is right here").
//
//  NOTHING IS DECIDED HERE. The recipient rules, the "is there anything to
//  send" rule and the attachment cap are MessageIntentPlan's, pinned by
//  tests; the send itself is MessageIntentRunner's, which is the composer's
//  own. This file is the framework's shape and nothing else — which is why
//  it imports AppIntents and nothing else does.
//
//  WHO RUNS WHERE. The project builds with MainActor as the default
//  isolation. The types the framework holds off-thread — the entity, the
//  query, the action list and the shared send — say `nonisolated`, and each
//  one that needs the app's objects hands the work back to the main actor,
//  as CallIntentHandler does. The two intents do not: their parameters are
//  the framework's mutable storage, which cannot be `nonisolated` at all,
//  and `perform()` is async, so running it on the main actor costs a hop
//  that the send needs anyway.
//

import AppIntents
import Foundation

/// The one bridge from the intent framework to the app's live objects.
///
/// Set once on the main actor by the composition root (FamilyConnectApp)
/// before any intent can run, and read from whatever context the framework
/// picked — the same one-way door AppDelegate's statics are, and weak for
/// the same reason: the app owns the runner, this only borrows it. Nil is a
/// launch whose SwiftData container failed, and says so rather than
/// pretending to have sent something.
enum MessageIntents {
    nonisolated(unsafe) static weak var runner: MessageIntentRunner?

    /// Tell the system the recipient list is not the one it last read.
    ///
    /// The phrases that name a person ("Send a message to Anna with
    /// Family") exist once per recipient, and the system builds them from
    /// `FamilyRecipientQuery.suggestedEntities()` — but only when asked.
    /// Without this call there are no per-person shortcuts at all, in
    /// Spotlight, in the Shortcuts app or for Siri.
    static func recipientsChanged() {
        FamilyShortcuts.updateAppShortcutParameters()
    }
}

// MARK: - The recipient

/// A person in the family, or the family chat, as a choice Shortcuts can
/// show, save and hand back.
nonisolated struct FamilyRecipientEntity: AppEntity {

    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Family recipient"
    static let defaultQuery = FamilyRecipientQuery()

    /// `MessageIntentPlan.RecipientID` — a member's handle, or the family
    /// chat's. The only thing the system promises to keep.
    let id: String
    /// What the picker shows.
    let name: String

    init(id: String, name: String) {
        self.id = id
        self.name = name
    }

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)")
    }
}

/// Where the picker's list comes from: the app's own roster, at the moment
/// the Shortcut is opened. No cache — a family that gained or lost a member
/// must not be a list out of last week, and the list is cheap.
nonisolated struct FamilyRecipientQuery: EntityQuery {

    func entities(for identifiers: [String]) async throws -> [FamilyRecipientEntity] {
        await MainActor.run {
            // The id stands in when the roster no longer answers — dropping
            // the row instead would be a system error about a missing
            // entity, where keeping it lets the plan name the real problem
            // (a Shortcut left pointing at a member who deleted their
            // account).
            identifiers.map { id in
                FamilyRecipientEntity(id: id, name: MessageIntents.runner?.name(forID: id) ?? id)
            }
        }
    }

    func suggestedEntities() async throws -> [FamilyRecipientEntity] {
        await MainActor.run {
            (MessageIntents.runner?.choices() ?? []).map {
                FamilyRecipientEntity(id: $0.id, name: $0.name)
            }
        }
    }

    func results() async throws -> [FamilyRecipientEntity] {
        try await suggestedEntities()
    }
}

// MARK: - The intents

/// Send a message, and anything the Shortcut is holding with it.
///
/// Both halves are optional on purpose: the same action covers "post this
/// text at 8am", "send the last screenshot" and "say what the sensor saw,
/// with the reading attached". An empty message with nothing attached is the
/// one thing that is refused (MessageIntentPlan rule 2).
struct SendMessageIntent: AppIntent {

    static var title: LocalizedStringResource { "Send a Message" }

    static var description: IntentDescription? {
        IntentDescription("Sends a message, and anything attached to it, to a family member or the family chat.")
    }

    /// The send needs no window, and a Shortcut on a schedule must not drag
    /// the app across the screen every hour.
    static var openAppWhenRun: Bool { false }

    @Parameter(title: "Recipient", requestValueDialog: "Who do you want to send it to?")
    var recipient: FamilyRecipientEntity?

    /// For the cases a picker cannot cover: a name another action produced
    /// (a row of a table, a text field, the output of "Ask for Input").
    /// The entity wins when both are set — it is exact.
    @Parameter(title: "Recipient name")
    var recipientName: String?

    @Parameter(title: "Message", requestValueDialog: "What do you want to say?")
    var message: String?

    @Parameter(title: "Attachments")
    var attachments: [IntentFile]?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        // Asked for rather than refused: said to Siri or tapped in
        // Spotlight, the action arrives with nothing filled in, and the
        // system can put the question (the recipient as a list out of the
        // roster). Where nobody is there to answer — an automation at
        // 03:00 — the same throw is the action's error, so the plan's
        // refusals still hold.
        if SendShortcut.lacksRecipient(recipient, recipientName) {
            throw $recipient.needsValueError()
        }
        if SendShortcut.isBlank(message), (attachments ?? []).isEmpty {
            throw $message.needsValueError()
        }
        return try await SendShortcut.send(
            recipient: recipient,
            recipientName: recipientName,
            body: message,
            files: attachments ?? [])
    }
}

/// Send files, with an optional caption — the door for "attach voice, or
/// photo, or any other file" (issue #118).
///
/// It is the same send as the one above, with the two halves asked for in
/// the order that matters: here the attachment is required, so Shortcuts
/// insists on one instead of offering an action that would only be refused.
struct SendAttachmentIntent: AppIntent {

    static var title: LocalizedStringResource { "Send an Attachment" }

    static var description: IntentDescription? {
        IntentDescription("Sends a photo, a video, a voice recording or any other file to a family member or the family chat.")
    }

    static var openAppWhenRun: Bool { false }

    @Parameter(title: "Recipient", requestValueDialog: "Who do you want to send it to?")
    var recipient: FamilyRecipientEntity?

    @Parameter(title: "Recipient name")
    var recipientName: String?

    @Parameter(title: "Attachments", requestValueDialog: "What do you want to send?")
    var attachments: [IntentFile]?

    @Parameter(title: "Message")
    var message: String?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        // As SendMessageIntent: a question where somebody can answer it.
        if SendShortcut.lacksRecipient(recipient, recipientName) {
            throw $recipient.needsValueError()
        }
        if (attachments ?? []).isEmpty {
            throw $attachments.needsValueError()
        }
        return try await SendShortcut.send(
            recipient: recipient,
            recipientName: recipientName,
            body: message,
            files: attachments ?? [])
    }
}

// MARK: - The one send both of them run

nonisolated enum SendShortcut {

    static func isBlank(_ text: String?) -> Bool {
        (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Neither the picker's entity nor a name: nobody was named at all.
    static func lacksRecipient(_ recipient: FamilyRecipientEntity?, _ name: String?) -> Bool {
        recipient == nil && isBlank(name)
    }

    /// Who was meant: the picker's id when there is one (exact), a name
    /// otherwise (matched against the roster, refusal rather than a guess).
    static func send(
        recipient: FamilyRecipientEntity?,
        recipientName: String?,
        body: String?,
        files: [IntentFile]
    ) async throws -> some IntentResult & ProvidesDialog {
        guard let runner = MessageIntents.runner else {
            throw MessageIntentRunner.Failure.refusal(.signedOut)
        }
        let meant: MessageIntentPlan.Recipient
        if let recipient {
            guard let resolved = MessageIntentPlan.RecipientID.recipient(from: recipient.id) else {
                throw MessageIntentRunner.Failure.refusal(.unknownRecipient)
            }
            meant = resolved
        } else if let named = recipientName?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !named.isEmpty {
            meant = .named(named)
        } else {
            throw MessageIntentRunner.Failure.refusal(.unknownRecipient)
        }

        let handed = files.map(Self.shortcutFile)
        // The runner is the app's own main-actor object (store, outbox,
        // socket), and this is the framework's thread — so the send is
        // handed over exactly the way CallIntentHandler hands its roster
        // resolution back. `.value` is where a refusal becomes the
        // Shortcut's error.
        let outcome = try await Task { @MainActor in
            try await runner.send(recipient: meant, body: body, files: handed)
        }.value
        // "Sent" only for what the server has. A message still in the
        // outbox when the runner stopped waiting is queued, and says so:
        // telling a 03:00 automation "Sent" about a row that then sits on
        // the phone until morning is the one answer worse than an error.
        let name = outcome.name.flatMap { $0.isEmpty ? nil : $0 }
        switch (outcome.delivered, name) {
        case (true, let name?):
            return .result(dialog: IntentDialog("Sent to \(name)."))
        case (true, nil):
            return .result(dialog: IntentDialog("Sent."))
        case (false, let name?):
            return .result(dialog: IntentDialog("It will be sent to \(name) as soon as there is a connection."))
        case (false, nil):
            return .result(dialog: IntentDialog("It will be sent as soon as there is a connection."))
        }
    }

    /// The bytes or the file, whichever Shortcuts actually has. The URL goes
    /// first: a 100 MB clip streamed from disk is what `MediaPrep` and the
    /// uploader are built for, and pulling it into `data` would be the one
    /// way to lose it on a phone that has to hold it anyway.
    static func shortcutFile(_ file: IntentFile) -> ShortcutFile {
        if let url = file.fileURL {
            return ShortcutFile(fileURL: url, data: nil, type: file.type, name: file.filename)
        }
        return ShortcutFile(fileURL: nil, data: file.data, type: file.type, name: file.filename)
    }
}

// MARK: - What the Shortcuts app lists

/// The two actions as App Shortcuts: what Spotlight, the Shortcuts app's
/// "Family" row and Siri offer with nothing set up by hand.
///
/// EVERY PHRASE NAMES THE APP, and that is a build rule, not a style: the
/// metadata step (`appintentsmetadataprocessor`) fails the target on a
/// phrase without `\(.applicationName)` — "Every App Shortcut utterance
/// should contain '${applicationName}'" — and a target that fails it ships
/// no App Intents metadata at all, which is neither action anywhere: not in
/// the Shortcuts app, not in Spotlight, not for Siri. The app's name is
/// "Family" (CFBundleDisplayName), so the first phrase is said "Send a
/// message with Family".
///
/// TWO PHRASES EACH. The bare one runs the action and lets it ask who and
/// what. The one with `\(\.$recipient)` exists once per family member and
/// once for the family chat — "Send a message to Anna with Family" — which
/// is the per-person row the calling side already has in Siri suggestions.
/// The system builds those from the recipient query when
/// `MessageIntents.recipientsChanged()` tells it to.
///
/// THE TRANSLATIONS ARE NOT IN `Localizable.xcstrings`. A phrase is looked
/// up in its own table, `AppShortcuts.xcstrings`, under the phrase with
/// `${applicationName}` and `${recipient}` spelled that way; all nine
/// languages are there, Serbian Latin generated from the Cyrillic like
/// every other string here (CONTRIBUTING.md §9). The first phrase of each
/// shortcut is the one the system shows as its name.
nonisolated struct FamilyShortcuts: AppShortcutsProvider {

    @AppShortcutsBuilder static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: SendMessageIntent(),
            phrases: [
                "Send a message with \(.applicationName)",
                "Send a message to \(\.$recipient) with \(.applicationName)",
            ],
            shortTitle: "Send a Message",
            systemImageName: "envelope.arrow.right.circle")
        AppShortcut(
            intent: SendAttachmentIntent(),
            phrases: [
                "Send an attachment with \(.applicationName)",
                "Send an attachment to \(\.$recipient) with \(.applicationName)",
            ],
            shortTitle: "Send an Attachment",
            systemImageName: "paperclip")
    }
}
