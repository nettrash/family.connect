//
//  TranscriptSection.swift
//  FamilyConnect
//
//  "Show text" under a voice note, an audio file or a video, and the text
//  itself once it has been given (docs/protocol.md, "Transcripts on
//  request"). A recording the server cannot send itself — a video, an Ogg
//  file, one over the ceiling — has its sound taken out on this device
//  and sent with the request (`TranscriptSound`), under the same "Getting
//  the text…"; what this device cannot do is said here, never hidden.
//
//  On request, one recording at a time, and the answer goes to whoever
//  asked: nothing is transcribed unasked and nothing is added to the
//  message for anybody else. The text comes back in the response, is kept
//  on this device (`TranscriptStore`) and is drawn here under the player —
//  selectable, so it can be copied, and folded away with "Hide text".
//
//  Whether the action is offered at all is `TranscriptDoor`'s, one rule
//  for the phone and the Mac. Asking needs the asker's consent to the
//  assistant, because the recording's SOUND goes to its provider: a member
//  who has not agreed is shown the existing consent sheet first, and the
//  server's own `assistant_consent_required` — a consent withdrawn on
//  another device — raises the same question instead of a failure, asking
//  again on a yes, exactly as the board's backdrop does.
//
//  The sheet is presented from here rather than from the conversation,
//  because a thread is itself a sheet over the conversation and a second
//  sheet cannot be raised from underneath the first.
//
//  Platform-free — the same block on iOS and macOS.
//

import SwiftUI

/// The message a recording belongs to, as far as asking for its text needs
/// it. `messageID` is the SERVER id, nil while the message is on its way.
nonisolated struct TranscriptSubject: Equatable, Sendable {
    let chatID: Int64
    let messageID: Int64?
    let senderID: Int64
}

struct TranscriptSection: View {
    let attachment: AttachmentDTO
    /// Nil where the caller has no message to name — then nothing is drawn.
    let subject: TranscriptSubject?
    /// Which balloon this sits in, for contrast — an own balloon is filled
    /// with the tint, so nothing here may be drawn in the accent colour.
    var isMine: Bool = false
    /// Which video of an album pile this is, counted among its videos —
    /// drawn as "Video 2" above the action, and only when anything is
    /// drawn at all. Nil for a lone recording.
    var videoNumber: Int? = nil

    @Environment(AttachmentStore.self) private var attachments
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var showAssistantConsent = false

    private var ink: Color { isMine ? .white : .accentColor }

    /// What the action is under this recording, read afresh every time —
    /// so the retry after a yes on the consent sheet sees the consent the
    /// yes just gave.
    private var door: TranscriptDoor {
        guard let subject else { return .absent }
        return TranscriptDoor.of(
            serverTranscribes: AppSettings.assistantTranscribe,
            maxBytes: AppSettings.assistantTranscribeMaxBytes,
            processor: AppSettings.assistantProcessor,
            agreedAt: session.assistantConsentAt,
            attachment: attachment,
            messageID: subject.messageID,
            chatKind: coordinator.chatKind(of: subject.chatID),
            senderID: subject.senderID,
            currentUserID: coordinator.currentUserID,
            assistantUserID: AppSettings.assistantUserID,
            familyAllowsTranscripts: session.family?.aiTranscripts == true)
    }

    var body: some View {
        let store = attachments.transcripts
        let kept = store.kept(attachment.id)
        let activity = store.activity(for: attachment.id)
        let door = self.door
        Group {
            if let videoNumber, kept != nil || door.isOffered {
                Text(verbatim: "\(String(localized: "Video")) \(videoNumber)")
                    .font(.caption2)
                    .opacity(0.75)
                    .padding(.horizontal, 8)
            }
            if let kept {
                // Text this device was given is drawn whatever the door says
                // now: the reader already has it, and a switch turned off
                // since does not take back what was read.
                if kept.hidden {
                    actionButton("Show text") {
                        store.setHidden(false, for: attachment.id)
                    }
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        textBlock(kept)
                        actionButton("Hide text") {
                            store.setHidden(true, for: attachment.id)
                        }
                    }
                }
            } else if door.isOffered {
                switch activity {
                case .idle:
                    actionButton("Show text", action: ask)
                case .asking:
                    HStack(spacing: 6) {
                        ProgressView()
                            .controlSize(.small)
                            .tint(ink)
                        Text("Getting the text…")
                            .font(.caption)
                            .opacity(0.8)
                    }
                    .padding(.horizontal, 8)
                    .accessibilityElement(children: .combine)
                case .failed(let failure):
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: failure.message)
                            .font(.caption)
                            .opacity(0.8)
                            .fixedSize(horizontal: false, vertical: true)
                        // A refusal by the provider's own filter, a
                        // recording the server will not read, or sound
                        // this device cannot take out or fit, offers no
                        // retry: asking again gets the same answer.
                        if failure.offersRetry {
                            actionButton("Show text", action: ask)
                        }
                    }
                    .padding(.horizontal, 8)
                    .frame(maxWidth: 260, alignment: .leading)
                }
            }
        }
        .sheet(isPresented: $showAssistantConsent) {
            AssistantConsentSheet(
                processor: AppSettings.assistantProcessor ?? "",
                familyHistory: session.family?.aiHistory == true,
                familyVision: session.family?.aiVision == true,
                onAgree: {
                    try await session.setAssistantConsent(true)
                    showAssistantConsent = false
                    ask()
                },
                onDecline: {
                    // "Not now" leaves the recording as it was.
                    showAssistantConsent = false
                })
        }
    }

    /// The text, or "No speech" for a recording with none in it — an
    /// answer, not an error. Selectable so it can be copied, and labelled
    /// so a screen reader does not read it out as the message itself.
    @ViewBuilder
    private func textBlock(_ kept: KeptTranscript) -> some View {
        Group {
            if kept.isSilence {
                Text("No speech")
                    .italic()
                    .opacity(0.75)
            } else {
                Text(verbatim: kept.text)
                    .textSelection(.enabled)
            }
        }
        .font(.callout)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: 260, alignment: .leading)
        .padding(.horizontal, 8)
        .padding(.top, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Text of the recording"))
        .accessibilityValue(
            kept.isSilence ? Text("No speech") : Text(verbatim: kept.text))
    }

    private func actionButton(
        _ title: LocalizedStringKey, action: @escaping () -> Void
    ) -> some View {
        Button(title, action: action)
            .font(.caption.weight(.medium))
            .foregroundStyle(ink)
            .buttonStyle(.borderless)
            .padding(.horizontal, 8)
    }

    /// Ask. Nothing reaches the provider unasked: a member who has not
    /// agreed is asked first and it is sent on a yes, and the server's own
    /// `assistant_consent_required` raises the same question instead of a
    /// failure (protocol.md, "Consenting to the assistant").
    private func ask() {
        guard let subject, let messageID = subject.messageID else { return }
        switch door {
        case .absent:
            return
        case .asksFirst:
            showAssistantConsent = true
            return
        case .open:
            break
        }
        let store = attachments.transcripts
        let attachment = self.attachment
        Task {
            let outcome = await store.ask(
                chatID: subject.chatID, messageID: messageID, attachment: attachment,
                maxBytes: AppSettings.assistantTranscribeMaxBytes)
            // The door is absent wherever the question cannot be asked (no
            // processor named), so a consent refusal always has a sheet.
            if outcome?.asksForConsent(processor: AppSettings.assistantProcessor) == true {
                showAssistantConsent = true
            }
        }
    }
}
