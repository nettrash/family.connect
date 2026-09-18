//
//  ReportSheet.swift
//  FamilyConnect
//
//  Reporting a member to the family owner (docs/protocol.md, "Reporting a
//  member"). Shared by iOS and macOS.
//
//  The four reasons are FIXED, and their raw values are the untranslated
//  wire strings. This is a nine-language product with a nine-language
//  owner: a free-text reason means an inbox the owner cannot read, and a
//  label sent as the reason means a `validation` refusal in whichever
//  language the reporter happens to use.
//

import SwiftUI

/// Which assistant reply is being reported (docs/protocol.md, "Reporting the
/// assistant"). No sender: the assistant is one account and naming it would
/// only invite somebody to pass it to the member-report endpoint, which
/// refuses it.
nonisolated struct AssistantReportTarget: Identifiable, Equatable {
    let messageID: Int64
    /// Whether the reply was in the reporter's own private thread. It changes
    /// nothing about where the report goes — the operator reads both — and is
    /// kept because the sheet says something slightly different about a
    /// reply the family could already read.
    let isPrivateThread: Bool

    var id: Int64 { messageID }
}

/// WHICH SAFETY ROW A BUBBLE GETS, decided in one place for both platforms.
///
/// The two reports are exclusive, and the exclusion is the protocol's: a
/// member report names somebody in your family and the OWNER reads it, while
/// an assistant report names a reply from an account that belongs to no
/// family at all and the OPERATOR reads it. Sending either down the other's
/// endpoint is refused — `not_same_family` one way, `message_not_found` the
/// other — so a menu that offered the wrong one would be a visible failure on
/// a safety screen.
nonisolated enum SafetyRules {
    /// A member's message, acked, not the reader's own and not the
    /// assistant's.
    static func canReportMember(
        senderID: Int64, currentUserID: Int64, isAssistant: Bool, hasServerID: Bool
    ) -> Bool {
        hasServerID && senderID != currentUserID && !isAssistant
    }

    /// An assistant reply, acked. Never the reader's own message, which in
    /// the assistant's own chat is the only other thing there is.
    static func canReportAssistant(
        senderID: Int64, currentUserID: Int64, isAssistant: Bool, hasServerID: Bool
    ) -> Bool {
        hasServerID && senderID != currentUserID && isAssistant
    }
}

/// Who is being reported, and optionally which of their messages.
nonisolated struct ReportTarget: Identifiable, Equatable {
    let senderID: Int64
    let senderName: String
    /// The message named, when the report came from a bubble rather than a
    /// roster row. `nil` reports the PERSON.
    let messageID: Int64?

    var id: Int64 { senderID }
}

/// The protocol's four, and only these.
///
/// The raw value goes on the wire untranslated; `label` is what a person
/// reads. Keeping them apart is what stops a translated string reaching
/// `POST /families/reports`.
nonisolated enum ReportReason: String, CaseIterable, Identifiable {
    case spam
    case harassment
    case inappropriate
    case other

    var id: String { rawValue }

    var label: String {
        switch self {
        case .spam:
            String(localized: "Spam", comment: "Report reason")
        case .harassment:
            String(localized: "Harassment", comment: "Report reason")
        case .inappropriate:
            String(localized: "Inappropriate", comment: "Report reason")
        case .other:
            String(localized: "Something else", comment: "Report reason")
        }
    }
}

struct ReportSheet: View {
    @Environment(AppSession.self) private var session
    let target: ReportTarget
    let onSubmit: (ReportReason) -> Void
    let onCancel: () -> Void

    @State private var reason: ReportReason = .harassment

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker(String(localized: "Reason", comment: "Report sheet: the picker's label"), selection: $reason) {
                        ForEach(ReportReason.allCases) { reason in
                            Text(reason.label).tag(reason)
                        }
                    }
                    #if os(macOS)
                        .pickerStyle(.inline)
                    #endif
                } header: {
                    Text("Why are you reporting \(target.senderName)?",
                         comment: "Report sheet header; %@ is the member's display name")
                } footer: {
                    // MANDATORY, and it is a protocol requirement rather
                    // than a nicety: "somebody who reports a message
                    // without knowing that has been surprised by their own
                    // app". It matters most in a DIRECT chat, where the
                    // owner has never seen the message before.
                    if target.messageID != nil {
                        Text(
                            "Your family owner will see this message and its text.",
                            comment: "Report sheet disclosure")
                    } else {
                        Text(
                            "Your family owner will be told you reported this member.",
                            comment: "Report sheet disclosure")
                    }
                }
                // The honest escalation path for the case this whole
                // feature is weakest at: the moderator IS the owner, so a
                // report about them never reaches them (docs/protocol.md,
                // "Reporting a member"). Absent when the operator has set
                // no contact, and then the section goes with it rather
                // than standing empty.
                if let supportContact = session.supportContact, !supportContact.isEmpty {
                    Section {
                        // VERBATIM, selectable, and never linkified: an
                        // operator may write an address, a URL or a whole
                        // sentence, and three apps guessing differently
                        // about which it is would be worse than three apps
                        // showing the same text.
                        Text(verbatim: supportContact)
                            .textSelection(.enabled)
                    } header: {
                        Text("If the problem is the owner", comment: "Report sheet: the operator's own contact")
                    } footer: {
                        Text(
                            "This server's operator published this contact.",
                            comment: "Report sheet: explains where the support contact came from")
                    }
                }
            }
            .navigationTitle(Text("Report", comment: "Report sheet title and its confirm button"))
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                        .keyboardShortcut(.cancelAction)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Report", comment: "Report sheet title and its confirm button")) { onSubmit(reason) }
                }
            }
        }
        // A Mac sheet cannot be resized by the person using it, so it is
        // sized here or it is wrong for everybody.
        #if os(macOS)
            .frame(width: 420, height: 320)
        #endif
    }
}

/// Reporting an ASSISTANT reply (docs/protocol.md, "Reporting the
/// assistant"). Shared by iOS and macOS.
///
/// Two things make it a different sheet rather than a flag on the one above.
/// It says WHO READS IT — the people who run the server, never the family
/// owner — and that is not a nicety: a private assistant thread belongs to
/// its member alone, so somebody reporting a reply out of one has to know
/// before they send it who is about to see it. And it takes a NOTE, which a
/// member report has none of: the reader here is one operator rather than a
/// nine-language owner, and "it invented a person" is not any of four words.
struct AssistantReportSheet: View {
    @Environment(AppSession.self) private var session
    let target: AssistantReportTarget
    let onSubmit: (ReportReason, String?) -> Void
    let onCancel: () -> Void

    @State private var reason: ReportReason = .inappropriate
    @State private var note: String = ""

    /// The server's cap, enforced here so nobody types past it and is
    /// refused (`POST /reports/assistant`).
    private static let noteLimit = 1000

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker(
                        String(localized: "Reason", comment: "Report sheet: the picker's label"),
                        selection: $reason
                    ) {
                        ForEach(ReportReason.allCases) { reason in
                            Text(reason.label).tag(reason)
                        }
                    }
                    #if os(macOS)
                        .pickerStyle(.inline)
                    #endif
                } header: {
                    Text("What was wrong with this reply?",
                         comment: "Assistant report sheet header")
                }

                Section {
                    TextField(
                        String(localized: "Say something about it (optional)",
                               comment: "Assistant report sheet: the note field"),
                        text: $note, axis: .vertical)
                        .lineLimit(3...6)
                        .onChange(of: note) { _, typed in
                            if typed.count > Self.noteLimit {
                                note = String(typed.prefix(Self.noteLimit))
                            }
                        }
                } footer: {
                    // MANDATORY, and for the reason the member sheet's
                    // disclosure is: somebody who reports a reply out of
                    // their own private thread without knowing who reads it
                    // has been surprised by their own app.
                    Text(
                        "The people who run this server will see this reply and what you write here. Your family will not.",
                        comment: "Assistant report sheet disclosure")
                }

                if let supportContact = session.supportContact, !supportContact.isEmpty {
                    Section {
                        // VERBATIM, selectable, never linkified — the same
                        // rule the member sheet follows.
                        Text(verbatim: supportContact)
                            .textSelection(.enabled)
                    } footer: {
                        Text(
                            "This server's operator published this contact.",
                            comment: "Report sheet: explains where the support contact came from")
                    }
                }
            }
            .navigationTitle(Text("Report this reply", comment: "Assistant report sheet title"))
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                        .keyboardShortcut(.cancelAction)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Report", comment: "Report sheet title and its confirm button")) {
                        let typed = note.trimmingCharacters(in: .whitespacesAndNewlines)
                        onSubmit(reason, typed.isEmpty ? nil : typed)
                    }
                }
            }
        }
        #if os(macOS)
            .frame(minWidth: 420, minHeight: 360)
        #endif
    }
}
