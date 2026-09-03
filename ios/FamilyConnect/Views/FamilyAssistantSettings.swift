//
//  FamilyAssistantSettings.swift
//  FamilyConnect
//
//  The two owner-only settings that decide how the assistant behaves in
//  the family chat: which language it answers in, and how much of the
//  chat it is shown when somebody mentions it.
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

        // Inline rather than in a footer: a refusal has to be visible on
        // both platforms, and only one of them draws footers on this
        // screen. Its own section now that there are three switches above
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

    // MARK: - The three sentences

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
    /// left on a table. The three sentences are the three facts a family
    /// needs to decide: where a photo may be attached, that each one is a
    /// separate deliberate act, and that the family chat is never a door.
    private var visionExplanation: String {
        String(localized: "With this on, anyone in the family can attach a photo to a question in their own chat with the assistant, and that photo is sent to the model your server is set up to use. It is sent only when they attach it to that question — never from the family chat, never from an earlier message, and never a video, file or place. With it off, no photo is ever sent.")
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
                save { try await coordinator.api.setAIVision(enabled) }
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
                    maxMembers: updated.maxMembers))
            } catch APIError.forbidden {
                errorText = String(localized: "Only the family owner can change this.")
            } catch {
                errorText = String(localized: "Couldn't save that. Try again.")
            }
        }
    }
}
