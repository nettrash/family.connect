//
//  FamilyAssistantSettings.swift
//  FamilyConnect
//
//  The owner-only settings that decide how the assistant behaves in the
//  family chat: which language it answers in, how much of the chat it is
//  shown when somebody mentions it, whether it may be shown a photograph
//  at all, and — the third switch — whether a mention may also be shown
//  the chat's most recent photographs.
//
//  Written once and dropped into both rosters — the phone's
//  FamilyManageView and the Mac's MacFamilyView — because they are the
//  same two questions with the same two consequences, and a family that
//  changed a setting on one device and could not find it on the other
//  would reasonably conclude the app had lost it. Sharing them also means
//  one catalogue key per sentence rather than two that drift apart in
//  translation.
//
//  What is NOT shared is where the explanation under each control sits.
//  The phone's screen is a Form and explains itself in section footers;
//  the Mac's is a List and explains itself in a caption line inside the
//  section, exactly as its invite code already does. Same words, each
//  screen's own idiom.
//
//  Both settings are owner endpoints on the server (protocol.md,
//  "PATCH /families/mine"); the caller is responsible for its own
//  `isOwner` gate, exactly as it already is for the invite code.
//
//  Android counterpart: ui/settings/FamilyAdminScreen.kt
//

import SwiftUI

struct FamilyAssistantSettings: View {
    /// The family as last read. The parent owns it, because it also owns
    /// the invite code and the join policy that sit next to these.
    let family: FamilyDTO
    /// The family the PATCH answered with — the parent writes it back
    /// wherever it holds one.
    var onUpdated: (FamilyDTO) -> Void

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @State private var isSaving = false
    @State private var errorText: String?

    var body: some View {
        Section {
            Picker("Answers in", selection: languageBinding) {
                // "Not set" is a real, reachable choice and NOT a synonym
                // for English: unset is on the wire, and it means the
                // family never chose (protocol.md, "The family's
                // language"). Each of the nine names itself, the way an OS
                // language picker does — somebody looking for their own
                // language scans for the word they recognise.
                Text("Not set").tag(String?.none)
                ForEach(FamilyLanguage.all) { language in
                    Text(language.name).tag(String?.some(language.tag))
                }
            }
            .pickerStyle(.menu)
            .disabled(isSaving)
            caption(languageExplanation)
        } header: {
            Text("Assistant language")
        } footer: {
            footer(languageExplanation)
        }

        Section {
            Toggle("Sees recent history", isOn: historyBinding)
                .disabled(isSaving)
            caption(historyExplanation)
        } footer: {
            footer(historyExplanation)
        }

        // ABSENT, not disabled, on a server whose operator configured no
        // deployment that can see. A switch that could never do anything is
        // a promise the server cannot keep, and a family reading it would
        // reasonably conclude their photographs were one tap from leaving
        // (protocol.md, "Pictures" — `assistant.vision` is exactly this
        // answer, and it is why the field exists).
        if AppSettings.assistantVision {
            Section {
                Toggle("Can be shown photos", isOn: visionBinding)
                    .disabled(isSaving)
                caption(visionExplanation)
            } header: {
                Text("Pictures")
            } footer: {
                footer(visionExplanation)
            }
        }

        // The THIRD switch, and unlike the one above it is PRESENT on a
        // server that cannot see — disabled, with the reason — because the
        // protocol asks for exactly that: one of its two reasons is
        // something the owner can act on ("turn Can be shown photos on
        // first", since the server refuses this while that is off) and the
        // other is something they deserve to be told rather than left to
        // find missing (protocol.md, "Recent photos from the family chat"
        // — "What a client shows"). Its own section, like "Sees recent
        // history", so each platform explains it in its own idiom.
        Section {
            Toggle("Recent photos", isOn: historyPhotosBinding)
                .disabled(isSaving || !historyPhotosSwitch.isEnabled)
            caption(historyPhotosExplanation)
        } footer: {
            footer(historyPhotosExplanation)
        }

        // Inline rather than in a footer: a refusal has to be visible on
        // both platforms, and only one of them draws footers on this
        // screen. Its own section now that there are four switches above
        // it — it belongs to whichever one just failed, and hanging it off
        // the last one would put it under the wrong sentence.
        if let errorText {
            Section {
                Label(errorText, systemImage: "exclamationmark.circle")
                    .font(.callout)
                    .foregroundStyle(.red)
            }
        }
    }

    // MARK: - The four sentences

    /// It is the assistant's language and not the app's, and the app's is
    /// the thing a reader will assume — so the difference is said out loud.
    private var languageExplanation: String {
        String(localized: "The language \(AssistantMention.token) answers in when it is asked in the family chat. It is not this app's language — that follows the device. With none chosen, it answers in the language of whoever asked.")
    }

    /// Plainly, because this is the setting that decides what leaves the
    /// server.
    private var historyExplanation: String {
        String(localized: "With this on, mentioning \(AssistantMention.token) in the family chat sends the last month of that chat to the assistant, so it can answer questions about what was said earlier. With it off, only the message that mentions it is sent.")
    }

    /// The one setting on this screen that decides whether PHOTOGRAPHS
    /// leave the server, so it says exactly what it does and exactly what
    /// it does not.
    ///
    /// Off by default where history is on, and the contrast is the point:
    /// history widened what a model was already being told, while a
    /// photograph is a different kind of thing entirely — faces of people
    /// who are not in the conversation, the inside of a home, a document
    /// left on a table. The sentence names the three doors a photo can go
    /// through — a question in a member's own assistant chat, an `@ai`
    /// message in the family chat, and an `@ai` reply to a photo — and
    /// that each of them is a member deliberately pointing the assistant
    /// at THAT picture. Until #56 it said "never from the family chat";
    /// that stopped being true when a mention was allowed to carry the
    /// photos on itself and on the message it quotes, and a switch whose
    /// description is wrong is worse than no description (protocol.md,
    /// "Showing the assistant a picture from the family chat"). What it
    /// still promises, unchanged: never a photo the assistant was not
    /// pointed at, and with it off, no photo at all.
    ///
    /// Changed once more with the third switch (protocol.md, "Recent
    /// photos from the family chat"): "never from an earlier message"
    /// gained "unless Recent photos is on" — the smallest change that
    /// keeps the sentence true, and the reason that widening is a switch
    /// of its own rather than a new meaning for this one. Every owner who
    /// turned this on did so under the old sentence; the new switch asks
    /// them a new question instead of quietly re-answering the old one.
    private var visionExplanation: String {
        String(localized: "With this on, a photo is sent to the model your server is set up to use when a member attaches it to a question in their own chat with the assistant, attaches it to an \(AssistantMention.token) message in the family chat, or replies to a photo with \(AssistantMention.token) — never a photo the assistant was not pointed at, never from an earlier message unless Recent photos is on, and never a video, file or place. With it off, no photo is ever sent.")
    }

    /// The third switch's state from the two locks under it — the shared
    /// rule, so the Mac and the phone cannot disagree on when it is
    /// withheld (`AssistantSurfaces.historyPhotosSwitch`).
    private var historyPhotosSwitch: AssistantSurfaces.HistoryPhotosSwitch {
        AssistantSurfaces.historyPhotosSwitch(
            serverCanSee: AppSettings.assistantVision,
            familyAllowsPhotos: family.aiVision)
    }

    /// What leaves, and what it costs — both, because the cost is the
    /// reason this defaults off (protocol.md, "Recent photos from the
    /// family chat": with `ai_history` on by default and a chat that is
    /// mostly photographs, nearly every mention becomes a four-image
    /// vision call, for questions that are not about a picture at all).
    /// Then, when the switch is withheld, the reason; and when it is
    /// offered but inert because "Sees recent history" is off, that too.
    private var historyPhotosExplanation: String {
        var text = String(localized: "With this on, whenever anyone mentions \(AssistantMention.token) in the family chat, the most recent photos in that chat — up to \(AssistantPictureLimits.maxPerQuestion), from anyone, that nobody pointed the assistant at — also go to the model your server is set up to use, after any photo on the message itself or on the one it replies to. Nearly every mention then sends pictures, which costs more. It is off unless you turn it on.")
        switch historyPhotosSwitch {
        case .offered:
            if !family.aiHistory {
                text += " " + String(localized: "While Sees recent history is off this does nothing: the chat's history isn't sent, so no photo from it is either.")
            }
        case .withheldNoVisionDeployment:
            text += " " + String(localized: "Not available here: the assistant on this server can't look at pictures.")
        case .withheldVisionOff:
            text += " " + String(localized: "Turn on Can be shown photos first — the server refuses this while that is off.")
        }
        return text
    }

    /// The Mac's spelling of an explanation — a caption inside the section.
    @ViewBuilder
    private func caption(_ text: String) -> some View {
        #if os(macOS)
        Text(text)
            .font(.caption)
            .foregroundStyle(.secondary)
        #endif
    }

    /// The phone's — a section footer.
    @ViewBuilder
    private func footer(_ text: String) -> some View {
        #if os(iOS)
        Text(text)
        #endif
    }

    // MARK: - Bindings

    private var languageBinding: Binding<String?> {
        Binding(
            get: { family.language },
            set: { tag in
                guard tag != family.language else { return }
                // nil here is a genuine CLEAR and travels as an explicit
                // JSON null — see APIClient.FamilyPatchRequest.
                save { try await coordinator.api.setFamilyLanguage(tag) }
            })
    }

    private var historyBinding: Binding<Bool> {
        Binding(
            get: { family.aiHistory },
            set: { enabled in
                guard enabled != family.aiHistory else { return }
                save { try await coordinator.api.setAIHistory(enabled) }
            })
    }

    private var visionBinding: Binding<Bool> {
        Binding(
            get: { family.aiVision },
            set: { enabled in
                guard enabled != family.aiVision else { return }
                // Turning this OFF turns "Recent photos" off in the same
                // write on the server; the family it answers with carries
                // both, and `save` writes both back.
                save { try await coordinator.api.setAIVision(enabled) }
            })
    }

    private var historyPhotosBinding: Binding<Bool> {
        Binding(
            get: { family.aiHistoryPhotos },
            set: { enabled in
                guard enabled != family.aiHistoryPhotos else { return }
                save { try await coordinator.api.setAIHistoryPhotos(enabled) }
            })
    }

    private func save(_ work: @escaping () async throws -> FamilyDTO) {
        isSaving = true
        errorText = nil
        Task {
            defer { isSaving = false }
            do {
                let updated = try await work()
                // PATCH answers with the family as it now stands; keep the
                // invite code we already hold in case this caller is one
                // the server did not send it to.
                onUpdated(FamilyDTO(
                    id: updated.id,
                    name: updated.name,
                    joinPolicy: updated.joinPolicy,
                    createdAt: updated.createdAt,
                    inviteCode: updated.inviteCode ?? family.inviteCode,
                    language: updated.language,
                    aiHistory: updated.aiHistory,
                    // The server's answer, as above — and this one is why
                    // the field is undefaulted in the initialiser: a
                    // rebuild that dropped it would turn the picture switch
                    // back off on screen while the server had it on.
                    aiVision: updated.aiVision,
                    // Likewise — and this one may have gone off without
                    // being asked, when `aiVision` did.
                    aiHistoryPhotos: updated.aiHistoryPhotos,
                    maxMembers: updated.maxMembers))
            } catch APIError.forbidden {
                errorText = String(localized: "Only the family owner can change this.")
            } catch {
                errorText = String(localized: "Couldn't save that. Try again.")
            }
        }
    }
}
