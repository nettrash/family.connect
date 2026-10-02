//
//  TranscriptStore.swift
//  FamilyConnect
//
//  The text of recordings this device asked for, kept per attachment, and
//  the requests out right now (docs/protocol.md, "Transcripts on request").
//
//  KEPT ON THIS DEVICE, so reopening a chat shows the text it was given
//  without asking again. A transcript is the ANSWER to one member's
//  request: it is not in the `Attachment` object, not in a history page and
//  not in a frame, so nothing else on this device will ever carry it, and
//  an answer made from sound this device SUPPLIED is not kept by the server
//  at all — this file is the only copy there is.
//
//  Owned by `AttachmentStore` and kept in a folder inside its directory,
//  for the reason the poster markers live there: a transcript is about one
//  attachment and lives and dies with it. `forget(attachmentIDs:)` (a chat
//  that went away) and `clear()` (logout) reach it through that store, so
//  the next account can never read the last one's words.
//
//  The text is NEVER logged — not the words, not the language.
//
//  A REQUEST BELONGS TO THE ACCOUNT THAT MADE IT. This store lives as long
//  as the app, across logouts, and a request can be out for minutes. Each
//  one carries a ticket; `clear()` and `forget(attachmentIDs:)` take the
//  tickets back, and an answer that lands without its ticket is dropped —
//  not kept, not drawn. Otherwise a request still out at logout would
//  write its answer back into the folder `clear()` just emptied, and the
//  next account would be shown the last one's words without asking, even
//  an answer made from sound this device supplied, which must never reach
//  anybody but the member who asked.
//
//  Android counterpart: none yet (#62 is per client).
//

import Foundation
import Observation
import os

/// One answer, as this device keeps it.
nonisolated struct KeptTranscript: Codable, Equatable, Sendable {
    /// Where the answer came from.
    enum Source: String, Codable, Sendable {
        /// The server's own copy of the recording. The server keeps this
        /// answer too and hands it to the next member who may ask.
        case stored
        /// Sound this device made and sent with the request. The server
        /// keeps nothing, so this device's copy is the only one.
        case supplied
    }

    let text: String
    let language: String?
    let source: Source
    /// The reader folded it away with "Hide text". Kept with the text so a
    /// fold survives a relaunch like the text does.
    var hidden: Bool = false

    /// "Nothing was said" — an answer, drawn as "No speech", never an error.
    var isSilence: Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// What the row under one recording is doing, apart from any kept text.
nonisolated enum TranscriptActivity: Equatable, Sendable {
    case idle
    /// The request is out: "Getting the text…".
    case asking
    /// It came back without a text, and why.
    case failed(TranscriptFailure)
}

@MainActor
@Observable
final class TranscriptStore {

    /// Makes the multipart `audio` part for an attachment the server cannot
    /// send itself: given the attachment, the server's ceiling and a fresh
    /// scratch folder (which the store removes afterwards), it returns the
    /// M4A to send — or throws `TranscriptSound.Failure` / a transport
    /// error. `TranscriptSound.prepare` in the app; a seam for the tests.
    typealias SoundSupplier = @Sendable (AttachmentDTO, Int64?, URL) async throws -> URL

    private let api: APIClient
    private let directory: URL
    private let supplySound: SoundSupplier

    /// Bumped on every write, so a row reading `kept(_:)` redraws when the
    /// answer lands. The disk memo below is ObservationIgnored — a lazy read
    /// from a view body must not count as a mutation.
    private(set) var revision = 0
    /// Per attachment, what is happening right now. Never on disk: a
    /// refusal is a fact about the server at the moment it answered, and an
    /// owner turning the switch on must not be hidden behind it after a
    /// relaunch.
    private var activities: [Int64: TranscriptActivity] = [:]
    /// What was read from disk, including "nothing there" (the inner nil),
    /// so a bubble redrawn on every scroll frame does not touch the disk.
    @ObservationIgnored private var memo: [Int64: KeptTranscript?] = [:]
    /// The ticket of the request out for each attachment. Taken back by
    /// `clear()` and `forget(attachmentIDs:)`, so an answer that lands for a
    /// request they ended is dropped.
    @ObservationIgnored private var tickets: [Int64: UInt64] = [:]
    @ObservationIgnored private var lastTicket: UInt64 = 0

    /// `directory` and `supplySound` are test seams, like
    /// `AttachmentStore`'s directory.
    init(api: APIClient, directory: URL, supplySound: SoundSupplier? = nil) {
        self.api = api
        self.directory = directory
        self.supplySound = supplySound ?? { attachment, maxBytes, folder in
            try await TranscriptSound.prepare(attachment, maxBytes: maxBytes, api: api, into: folder)
        }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    private func fileURL(_ attachmentID: Int64) -> URL {
        directory.appendingPathComponent("\(attachmentID)").appendingPathExtension("json")
    }

    // MARK: - Reading

    /// The answer this device holds for an attachment, or nil.
    func kept(_ attachmentID: Int64) -> KeptTranscript? {
        _ = revision
        if let known = memo[attachmentID] { return known }
        let url = fileURL(attachmentID)
        var value: KeptTranscript?
        if let data = try? Data(contentsOf: url) {
            value = try? JSONDecoder().decode(KeptTranscript.self, from: data)
            // Unreadable is as good as absent — and must not stay to be
            // tripped over on every redraw.
            if value == nil { try? FileManager.default.removeItem(at: url) }
        }
        memo[attachmentID] = .some(value)
        return value
    }

    func activity(for attachmentID: Int64) -> TranscriptActivity {
        activities[attachmentID] ?? .idle
    }

    // MARK: - Asking

    /// Ask the server for the text of one recording's STORED copy.
    ///
    /// Nil — and nothing is sent — while a request for the same attachment
    /// is already out: the wait can be minutes, and the second answer would
    /// only be the first one again. A text is kept on this device before it
    /// is returned; a consent refusal leaves the row idle (the caller asks
    /// the question and calls this again on a yes); any other refusal stays
    /// on the row until it is asked again.
    @discardableResult
    func ask(chatID: Int64, messageID: Int64, attachmentID: Int64) async -> TranscriptOutcome? {
        await ask(chatID: chatID, messageID: messageID, attachmentID: attachmentID) {
            (try await self.api.transcript(
                chatID: chatID, messageID: messageID, attachmentID: attachmentID), .stored)
        }
    }

    /// Ask for the text of one recording by whichever route it takes
    /// (`TranscriptDoor.route`): the stored copy, or sound this device
    /// takes out of the file and sends — downloaded, extracted, measured
    /// and uploaded, all under the one "Getting the text…". Nil, with
    /// nothing done, for an attachment with no sound to send or while a
    /// request for it is already out.
    ///
    /// An answer from SUPPLIED sound is kept here as `.supplied`: the
    /// server kept nothing, so this device's copy is the only one.
    ///
    /// A stored-copy request the server answers `not_transcribable` — its
    /// own reading of the file disagreeing with the attachment's metadata —
    /// goes on by supplied sound under the same "Getting the text…", as
    /// the protocol allows ("a client that sent no body may send the sound
    /// track instead") and as every other client does.
    @discardableResult
    func ask(
        chatID: Int64, messageID: Int64, attachment: AttachmentDTO, maxBytes: Int64?
    ) async -> TranscriptOutcome? {
        guard let route = TranscriptDoor.route(for: attachment, maxBytes: maxBytes) else {
            return nil
        }
        let api = self.api
        let supply = self.supplySound
        let supplied: () async throws -> TranscriptDTO = {
            let folder = FileManager.default.temporaryDirectory
                .appendingPathComponent("fc-transcript-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: folder) }
            let sound = try await supply(attachment, maxBytes, folder)
            return try await api.transcript(
                chatID: chatID, messageID: messageID, attachmentID: attachment.id,
                suppliedSound: sound)
        }
        return await ask(chatID: chatID, messageID: messageID, attachmentID: attachment.id) {
            switch route {
            case .supplied:
                return (try await supplied(), .supplied)
            case .stored:
                do {
                    return (try await api.transcript(
                        chatID: chatID, messageID: messageID, attachmentID: attachment.id), .stored)
                } catch APIError.conflict(let code, _) where code == "not_transcribable" {
                    return (try await supplied(), .supplied)
                }
            }
        }
    }

    /// `request` answers with the text and the form that produced it, which
    /// decides how it is kept.
    private func ask(
        chatID: Int64, messageID: Int64, attachmentID: Int64,
        request: () async throws -> (TranscriptDTO, TranscriptRoute)
    ) async -> TranscriptOutcome? {
        guard activity(for: attachmentID) != .asking else { return nil }
        activities[attachmentID] = .asking
        lastTicket &+= 1
        let ticket = lastTicket
        tickets[attachmentID] = ticket
        let result: Result<(TranscriptDTO, TranscriptRoute), Error>
        do {
            result = .success(try await request())
        } catch {
            result = .failure(error)
        }
        // Logged out, or the chat went away, while it was out: the answer
        // is not this account's to keep, and nothing is drawn.
        guard tickets[attachmentID] == ticket else { return nil }
        tickets[attachmentID] = nil
        let outcome: TranscriptOutcome
        switch result {
        case .success(let (answer, route)):
            keep(answer, source: route == .stored ? .stored : .supplied, for: attachmentID)
            outcome = .text(answer)
        case .failure(let error):
            outcome = TranscriptOutcome(error: error)
            // The outcome's NAME only — never anything the server said.
            AppLog.api.info("Transcript for attachment \(attachmentID, privacy: .public) not given")
        }
        switch outcome {
        case .text, .consentRequired:
            activities[attachmentID] = nil
        case .failed(let failure):
            activities[attachmentID] = .failed(failure)
        }
        return outcome
    }

    // MARK: - Writing

    /// Keep an answer for an attachment, shown (not folded).
    func keep(_ answer: TranscriptDTO, source: KeptTranscript.Source, for attachmentID: Int64) {
        write(
            KeptTranscript(text: answer.text, language: answer.language, source: source),
            for: attachmentID)
    }

    /// "Hide text" / show it again. The text stays on the device either way.
    func setHidden(_ hidden: Bool, for attachmentID: Int64) {
        guard var value = kept(attachmentID), value.hidden != hidden else { return }
        value.hidden = hidden
        write(value, for: attachmentID)
    }

    private func write(_ value: KeptTranscript, for attachmentID: Int64) {
        if let data = try? JSONEncoder().encode(value) {
            try? FileManager.default.createDirectory(
                at: directory, withIntermediateDirectories: true)
            try? data.write(to: fileURL(attachmentID), options: .atomic)
        }
        // In memory whatever the disk said: the reader was given this text,
        // and losing it to a full disk would be worse than not keeping it.
        memo[attachmentID] = .some(value)
        revision &+= 1
    }

    // MARK: - Forgetting

    /// The attachments are gone from this device (their chat went away).
    func forget(attachmentIDs: [Int64]) {
        guard !attachmentIDs.isEmpty else { return }
        for id in attachmentIDs {
            memo[id] = nil
            activities[id] = nil
            tickets[id] = nil
            try? FileManager.default.removeItem(at: fileURL(id))
        }
        revision &+= 1
    }

    /// Logout: every kept text goes, files and all.
    func clear() {
        memo.removeAll()
        activities.removeAll()
        tickets.removeAll()
        try? FileManager.default.removeItem(at: directory)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        revision &+= 1
    }
}
