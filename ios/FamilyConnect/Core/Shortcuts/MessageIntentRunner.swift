//
//  MessageIntentRunner.swift
//  FamilyConnect
//
//  The door between a Shortcut and the ordinary send (issue #118).
//
//  Everything a send touches lives on this side of the door: the store, the
//  outbox, the socket, the media rules. On the other side (MessageIntents)
//  is the intent framework, which the simulator cannot test and which knows
//  nothing about any of them. What crosses is a recipient, some text and a
//  few files going one way, and the name to say back coming the other.
//
//  WHY THE SEND IS NOT REIMPLEMENTED HERE. `ChatSyncCoordinator.send` and
//  `sendMedia` already write the row and its item rows BEFORE a byte leaves
//  the device, move the files where the system will not reclaim them, and
//  finish the job on whatever trigger comes next (docs/protocol.md,
//  "Sending on an unreliable network"). That is exactly what a Shortcut
//  running on a schedule at 3am needs — the app may be killed the moment
//  the send is queued, and the message still goes — so reusing it is not
//  merely tidy, it is the feature. A Shortcut that runs while the phone is
//  locked therefore behaves like a send made by hand and abandoned halfway.
//
//  Nothing here asks the user anything: a Shortcut has no window, and the
//  share extension's "nothing auto-sends" rule (ShareImport) is about the
//  composer a share lands in, which this path has none of. The
//  deliberate-sending half is done by whoever built the Shortcut, once, and
//  the refusals below are the guards.
//

import Foundation
import os
import UniformTypeIdentifiers

/// One file a Shortcut handed over, reduced to what `MediaPrep` needs.
///
/// `fileURL` and `data` are the two shapes Shortcuts delivers a file in —
/// a file it can name, or bytes in a variable — and either alone is enough.
nonisolated struct ShortcutFile {
    let fileURL: URL?
    let data: Data?
    /// What the sender says it is, when it says anything. `MediaPrep` trusts
    /// the caller over the extension, so a file that arrives through a
    /// variable keeps the type it had.
    let type: UTType?
    let name: String?
}

/// The Shortcuts send, run against the live session.
@MainActor
final class MessageIntentRunner {

    /// Why the Shortcut was told no. The sentences are in the conformance
    /// below, where the failure is the only thing that knows them.
    enum Failure: Error {
        case refusal(MessageIntentPlan.Refusal)
        /// It could not be read, decoded, or brought under the ceiling.
        case unreadable
        /// The direct chat could not be created (offline, blocked, gone).
        case couldNotOpenChat
        /// The composer's own queue refused it — an empty body with nothing
        /// staged, which the plan has already ruled out, so: a bug worth
        /// surfacing rather than reporting success.
        case refusedByStore
    }

    // MARK: - Collaborators
    //
    //  Closures rather than the objects themselves, exactly as CallManager
    //  and CallIntentHandler are wired: this type meets the store, the
    //  session and the coordinator only through what the composition root
    //  hands it (FamilyConnectApp), so it stays free of SwiftData, of the
    //  socket and of the intent framework.

    /// The gated roster: not me, not left, not deleted, never the assistant.
    var roster: () -> [CallRequestRouter.Candidate] = { [] }
    var isActiveMember: (Int64) -> Bool = { _ in false }
    var isBlocked: (Int64) -> Bool = { _ in false }
    var directChatID: (Int64) -> Int64? = { _ in nil }
    var familyChatID: () -> Int64? = { nil }
    /// The family chat's title — the name the picker shows for it and Siri
    /// says back.
    var familyChatTitle: () -> String? = { nil }
    var memberName: (Int64) -> String? = { _ in nil }
    var isSignedIn: () -> Bool = { false }
    /// Bootstrap and connect if this launch did not: a Shortcut can start
    /// this process with no window, no bootstrap and no socket.
    var ensureReady: () async -> Void = {}
    var openDirectChat: (Int64) async throws -> Int64 = { _ in
        throw Failure.couldNotOpenChat
    }
    var sendText: (String, Int64) -> String? = { _, _ in nil }
    var sendFiles: ([MediaPrep.Prepared], String, Int64) -> String? = { _, _, _ in nil }

    // MARK: - The picker

    /// What Shortcuts offers for "Recipient": the family chat, then the
    /// members. Empty while signed out, which is the honest list.
    func choices() -> [MessageIntentPlan.Choice] {
        MessageIntentPlan.choices(familyChatTitle: familyChatTitle(), roster: roster())
    }

    /// The name behind a picker id, for the entity the system shows again
    /// when it reviews or disambiguates a Shortcut.
    func name(forID id: String) -> String? {
        choices().first { $0.id == id }?.name
    }

    // MARK: - Sending

    /// Send what the Shortcut asked for.
    ///
    /// Returns the recipient's name for Siri to say ("Sent to Anna."), or
    /// nil when this device cannot name them — the dialog then says only
    /// that it went.
    func send(
        recipient: MessageIntentPlan.Recipient,
        body: String?,
        files: [ShortcutFile]
    ) async throws -> String? {
        // FIRST, so that the plan's "signed out" means signed out and not
        // merely "this background launch has not read the keychain yet".
        await ensureReady()
        let trimmed = body?.trimmingCharacters(in: .whitespacesAndNewlines)
        let decision = MessageIntentPlan.decide(
            recipient: recipient,
            body: trimmed,
            attachmentCount: files.count,
            signedIn: isSignedIn(),
            in: MessageIntentPlan.Directory(
                isActiveMember: isActiveMember,
                isBlocked: isBlocked,
                roster: roster,
                directChatID: directChatID,
                familyChatID: familyChatID))

        // Prepared BEFORE any chat is opened: a Shortcut whose photo turns
        // out to be unreadable must not leave behind a direct chat that was
        // created for a message that never arrived.
        var prepared: [MediaPrep.Prepared] = []
        do {
            for file in files { prepared.append(try await Self.prepare(file)) }
        } catch {
            for item in prepared { MediaPrep.discard(item) }
            throw error
        }

        let chatID: Int64
        var label: String?
        switch decision {
        case .refuse(let refusal):
            for item in prepared { MediaPrep.discard(item) }
            throw Failure.refusal(refusal)
        case .send(let id, let peerUserID):
            chatID = id
            label = peerUserID.flatMap { memberName($0) } ?? familyChatTitle()
        case .openDirectThenSend(let userID):
            guard let id = try? await openDirectChat(userID) else {
                for item in prepared { MediaPrep.discard(item) }
                AppLog.app.info("Shortcut send stopped: the direct chat could not be opened")
                throw Failure.couldNotOpenChat
            }
            chatID = id
            label = memberName(userID) ?? familyChatTitle()
        }

        let text = trimmed ?? ""
        let queued = prepared.isEmpty
            ? sendText(text, chatID)
            : sendFiles(prepared, text, chatID)
        guard queued != nil else {
            AppLog.app.info("Shortcut send stopped: the queue refused the message")
            throw Failure.refusedByStore
        }
        return label
    }

    /// One file from Shortcuts, made exactly the way the composer makes a
    /// picked one: a photo downscaled with a preview, a clip brought to the
    /// profile, audio under the audio rules, and anything else as itself.
    /// `MediaPrep` owns those decisions and the ceiling; nothing here
    /// re-decides them.
    ///
    /// `animates: true` because a Shortcut can only aim at a family or a
    /// direct chat, never the assistant's, and in those an animated GIF or
    /// WebP is an animation (docs/protocol.md, "Animations", #96).
    private static func prepare(_ file: ShortcutFile) async throws -> MediaPrep.Prepared {
        if let url = file.fileURL {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            do {
                return try await MediaPrep.prepare(
                    fileAt: url, type: file.type, name: file.name,
                    limit: MediaPrep.sizeLimit, animates: true)
            } catch {
                AppLog.app.info("Shortcut attachment refused: it could not be prepared")
                throw Failure.unreadable
            }
        }
        guard let data = file.data else { throw Failure.unreadable }
        do {
            return try await MediaPrep.prepare(
                data: data, type: file.type ?? .data, name: file.name,
                limit: MediaPrep.sizeLimit, animates: true)
        } catch {
            AppLog.app.info("Shortcut attachment refused: it could not be prepared")
            throw Failure.unreadable
        }
    }
}

// MARK: - What the user is told

/// The refusals in the user's words, in the one place that knows which one
/// happened. Shortcuts shows this as the action's error, and Siri says it.
extension MessageIntentRunner.Failure: CustomLocalizedStringResourceConvertible {

    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .refusal(.signedOut):
            return "Sign in to Family first, then run this Shortcut again."
        case .refusal(.nothingToSend):
            return "There is nothing to send. Add a message or an attachment."
        case .refusal(.tooManyAttachments):
            return "Too many attachments for one message."
        case .refusal(.unknownRecipient):
            return "Nobody in your family matches that recipient. Pick them again in Shortcuts."
        case .refusal(.ambiguous):
            return "Two family members share that name. Pick one of them in Shortcuts."
        case .refusal(.blocked):
            return "You've blocked that member. Unblock them from the family roster to send them a message."
        case .refusal(.noFamilyChat):
            return "This device has no family chat to send to."
        case .unreadable:
            return "One of the attachments could not be read."
        case .couldNotOpenChat:
            return "The chat could not be opened. Check your connection and try again."
        case .refusedByStore:
            return "The message could not be queued."
        }
    }
}
