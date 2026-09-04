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
