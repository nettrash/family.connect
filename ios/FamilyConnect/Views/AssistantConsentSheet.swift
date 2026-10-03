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
//  It does not save anything itself. The buttons hand the answer back and
//  the caller writes it through `AppSession.agreeToAssistant`, which is
//  what holds the server's own timestamps.
//
//  Where the server can look things up (`assistant.lookups`), the screen
//  asks a SECOND question beside the first — whether the assistant may send
//  a query it wrote to those providers — with "Agree With Lookups" and
//  "Agree Without Lookups" in place of "I Agree" (protocol.md, "Consenting
//  to the assistant", amended 2026-10-03). Settings raises it once more
//  for somebody who agreed to the assistant already, asking only that.
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
    /// Whether this server can turn a recording into text, which adds the
    /// line saying a recording's sound goes too when its text is asked for
    /// (protocol.md, "Transcripts on request"). Read from the server's last
    /// answer unless a caller says otherwise, so every place that raises
    /// this sheet says it.
    var transcribes: Bool = AppSettings.assistantTranscribe
    /// The providers the assistant may look things up in
    /// (`assistant.lookups`), read from the server's last answer like
    /// `transcribes`. Nil — no lookup source — leaves the screen exactly as
    /// it was before lookups existed.
    var lookups: [String]? = AppSettings.assistantLookups
    /// True when the member agreed to the assistant already and this
    /// screen asks only about lookups (Settings' "Review and Allow
    /// Lookups…"). Every screen that raises this on the way to sending a
    /// message raises it for somebody who has NOT agreed, so false is the
    /// default.
    var assistantAgreed: Bool = false
    /// Records the agreement — and, when the sheet was raised by a
    /// message waiting to go, sends it. Async and throwing so the sheet
    /// can keep the person here and say what went wrong instead of
    /// closing on a failure they would only notice as silence. Handed
    /// WHAT was agreed to, so the caller writes exactly that through
    /// `AppSession.agreeToAssistant`.
    let onAgree: (AssistantConsent.Answer) async throws -> Void
    let onDecline: () -> Void

    @State private var saving = false
    @State private var errorText: String?

    private var mode: AssistantConsent.SheetMode {
        AssistantConsent.sheetMode(assistantAgreed: assistantAgreed, lookups: lookups)
    }

    var body: some View {
        NavigationStack {
            Form {
                if case .lookupsOnly(let providers) = mode {
                    Section {
                        disclosureLine(
                            AssistantConsent.lookupDisclosure(
                                providers: providers, familyHistory: familyHistory),
                            systemImage: "magnifyingglass.circle")
                        // The existing "you can stop" line, verbatim: the
                        // same promise, and the same catalogue key.
                        disclosureLine(
                            String(localized: "You can stop this at any time in Settings. What has already been sent cannot be taken back."),
                            systemImage: "arrow.up.forward.circle")
                    } header: {
                        Text("Looking things up")
                    } footer: {
                        policyLink
                    }
                } else {
                    Section {
                        ForEach(
                            AssistantConsent.disclosure(
                                processor: processor,
                                familyHistory: familyHistory,
                                familyVision: familyVision,
                                transcribes: transcribes
                            ), id: \.self
                        ) { line in
                            disclosureLine(line, systemImage: "arrow.up.forward.circle")
                        }
                    } header: {
                        Text("Before the assistant answers")
                    } footer: {
                        if case .assistant = mode { policyLink }
                    }
                    if case .assistantWithLookups(let providers) = mode {
                        // Its own section, headed, because it is a
                        // different recipient and a different answer: the
                        // member may say yes to the assistant and no to
                        // this (protocol.md, "Consenting to the assistant",
                        // amended 2026-10-03).
                        Section {
                            disclosureLine(
                                AssistantConsent.lookupDisclosure(
                                    providers: providers, familyHistory: familyHistory),
                                systemImage: "magnifyingglass.circle")
                        } header: {
                            Text("Looking things up")
                        } footer: {
                            policyLink
                        }
                        // Two ways to say yes, side by side and equally
                        // plain — neither is the default, because neither
                        // is the answer this screen wants. "Not Now" stays
                        // where it always was.
                        Section {
                            Button("Agree With Lookups") { agree(.assistantAndLookups) }
                                .disabled(saving)
                            Button("Agree Without Lookups") { agree(.assistant) }
                                .disabled(saving)
                        }
                    }
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
                // One "I Agree" where there is one question; none where
                // there are two, whose buttons are in the form above.
                if let answer = toolbarAnswer {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("I Agree") { agree(answer) }
                            .disabled(saving)
                    }
                }
            }
        }
        // A Mac sheet cannot be resized by the person reading it, so it is
        // sized here or it is wrong for everybody — taller when it asks
        // two questions.
        #if os(macOS)
            .frame(width: 460, height: macHeight)
        #endif
    }

    /// What the toolbar's "I Agree" agrees to, or nil when the screen asks
    /// two questions and answers them with the two buttons in the form.
    private var toolbarAnswer: AssistantConsent.Answer? {
        switch mode {
        case .assistant: .assistant
        case .lookupsOnly: .lookups
        case .assistantWithLookups: nil
        }
    }

    #if os(macOS)
    private var macHeight: CGFloat {
        switch mode {
        case .assistant: 420
        case .assistantWithLookups: 580
        case .lookupsOnly: 320
        }
    }
    #endif

    private func disclosureLine(_ line: String, systemImage: String) -> some View {
        Label {
            Text(verbatim: line)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
        }
    }

    /// The policy is still linked, because the guideline asks for both:
    /// the disclosure where the answer is given, and a policy that holds
    /// the same promises.
    private var policyLink: some View {
        Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/privacy.html")!) {
            Label("Privacy Policy", systemImage: "hand.raised")
        }
        .font(.footnote)
        .padding(.top, 4)
    }

    private func agree(_ answer: AssistantConsent.Answer) {
        saving = true
        errorText = nil
        Task {
            do {
                try await onAgree(answer)
            } catch {
                errorText = String(
                    localized: "Couldn't save your answer. Try again.")
            }
            saving = false
        }
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
