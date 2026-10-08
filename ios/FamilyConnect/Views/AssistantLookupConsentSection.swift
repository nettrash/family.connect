//
//  AssistantLookupConsentSection.swift
//  FamilyConnect
//
//  The member's own answer to the SECOND assistant question — whether the
//  assistant may send a short query or place name it wrote from their
//  question to the lookup providers — and the way back out of it
//  (docs/protocol.md, "Consenting to the assistant", amended 2026-10-03,
//  and "Looking things up"). Shared by the phone's Settings and the Mac's,
//  directly under their "Assistant" section, so the two cannot disagree
//  about when it is drawn.
//
//  ABSENT unless there is something to agree to and the member has agreed
//  to the assistant already (`AssistantConsent.lookupSettings`): before
//  that, the assistant section's own "Review and Agree…" asks both on one
//  screen. Withdrawal is a plain button, as the first consent's is, because
//  stopping has nothing to read first.
//

import SwiftUI

struct AssistantLookupConsentSection: View {
    @Environment(AppSession.self) private var session
    /// Raise the consent screen; the caller owns the sheet, which asks only
    /// about lookups for somebody who agreed to the assistant already.
    let onReview: () -> Void

    @State private var errorText: String?

    private var state: AssistantConsent.LookupSettings {
        AssistantConsent.lookupSettings(
            processor: AppSettings.assistantProcessor,
            lookups: AppSettings.assistantLookups,
            assistantAgreedAt: session.assistantConsentAt,
            lookupAgreedAt: session.assistantLookupConsentAt)
    }

    var body: some View {
        switch state {
        case .absent:
            EmptyView()
        case .notAgreed(let providers):
            Section {
                Button("Review and Allow Lookups…", action: onReview)
                failure
            } header: {
                Text("Looking things up")
            } footer: {
                Text("Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to \(providers).")
            }
        case .agreed(let at, let providers):
            Section {
                LabeledContent(
                    String(localized: "Agreed"),
                    value: at.formatted(date: .abbreviated, time: .shortened))
                Button("Stop Lookups", role: .destructive) {
                    Task { await stop() }
                }
                failure
            } header: {
                Text("Looking things up")
            } footer: {
                Text("The assistant may send a short search query or place name it writes from your questions to \(providers). Stopping takes effect at once; what has already been sent cannot be taken back.")
            }
        }
    }

    @ViewBuilder
    private var failure: some View {
        if let errorText {
            Label(errorText, systemImage: "xmark.circle")
                .foregroundStyle(.red)
        }
    }

    /// Withdraw through the session, which holds the server's own answer. A
    /// failure is SHOWN: somebody who pressed stop and saw nothing change
    /// would reasonably believe it had.
    private func stop() async {
        do {
            try await session.setAssistantLookupConsent(false)
            errorText = nil
        } catch {
            errorText = String(localized: "Couldn't save your answer. Try again.")
        }
    }
}
