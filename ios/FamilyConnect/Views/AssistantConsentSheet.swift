//
//  AssistantConsentSheet.swift
//  FamilyConnect
//
//  Asking, once, before anything a member writes goes to the model
//  (docs/protocol.md, "Consenting to the assistant"). Shared by iOS and
//  macOS.
//
//  Everything the person needs is ON THIS SCREEN and not only behind the
//  policy link: who receives the words, what travels with them, that the
//  answer lands in the chat for everyone in it to read, and that stopping
//  later cannot recall what has already been sent. That is App Store
//  Review Guideline 5.1.1(i) and it is also the only version of this
//  screen worth showing — a person cannot weigh "a third party".
//
//  It does not save anything itself. The two buttons hand the answer back
//  and the caller writes it through `AppSession`, which is what holds the
//  server's own timestamp.
//

import SwiftUI

struct AssistantConsentSheet: View {
    /// Who answers, verbatim as the operator wrote it. The sheet is never
    /// presented without one — see `AssistantConsent.isAvailable`.
    let processor: String
    /// Whether an `@ai` in the family chat takes that chat's recent
    /// history with it (`ai_history`), which changes what this screen
    /// promises rather than merely how much it says.
    let familyHistory: Bool
    /// Whether a photograph may be shown to the model at all
    /// (`ai_vision`), for the same reason.
    let familyVision: Bool
    /// Records the agreement — and, when the sheet was raised by a
    /// message waiting to go, sends it. Async and throwing so the sheet
    /// can keep the person here and say what went wrong instead of
    /// closing on a failure they would only notice as silence.
    let onAgree: () async throws -> Void
    let onDecline: () -> Void

    @State private var saving = false
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(
                        AssistantConsent.disclosure(
                            processor: processor,
                            familyHistory: familyHistory,
                            familyVision: familyVision
                        ), id: \.self
                    ) { line in
                        Label {
                            Text(verbatim: line)
                                .font(.callout)
                                .fixedSize(horizontal: false, vertical: true)
                        } icon: {
                            Image(systemName: "arrow.up.forward.circle")
                                .foregroundStyle(.secondary)
                        }
                    }
                } header: {
                    Text("Before the assistant answers")
                } footer: {
                    // The policy is still linked, because the guideline
                    // asks for both: the disclosure where the answer is
                    // given, and a policy that holds the same promises.
                    Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/privacy.html")!) {
                        Label("Privacy Policy", systemImage: "hand.raised")
                    }
                    .font(.footnote)
                    .padding(.top, 4)
                }
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "xmark.circle")
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle(Text("The Assistant"))
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Not Now", action: onDecline)
                        .keyboardShortcut(.cancelAction)
                        .disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("I Agree") {
                        saving = true
                        errorText = nil
                        Task {
                            do {
                                try await onAgree()
                            } catch {
                                errorText = String(
                                    localized: "Couldn't save your answer. Try again.")
                            }
                            saving = false
                        }
                    }
                    .disabled(saving)
                }
            }
        }
        // A Mac sheet cannot be resized by the person reading it, so it is
        // sized here or it is wrong for everybody.
        #if os(macOS)
            .frame(width: 460, height: 420)
        #endif
    }
}

/// The one line a composer shows when the question has not been answered
/// yet — small, above the field, with the door on it. Shared, because the
/// phone and the Mac must not disagree about when it appears.
struct AssistantConsentBar: View {
    let processor: String
    let onReview: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: "hand.raised")
                .font(.caption)
                .foregroundStyle(.secondary)
            Text("This goes to \(processor). You haven't agreed to that yet.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button("Review…", action: onReview)
                .font(.caption)
                .buttonStyle(.borderless)
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .accessibilityElement(children: .combine)
    }
}
