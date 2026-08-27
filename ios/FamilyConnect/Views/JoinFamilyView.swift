//
//  JoinFamilyView.swift
//  FamilyConnect
//
//  Join by invite code. Codes are short uppercase tokens ("ABCD2345"),
//  so the field uppercases as the user types — people read them off
//  another person's screen and shouldn't have to fight autocapitalization
//  either way. Three outcomes per protocol.md POST /families/join:
//  "joined" → straight to the chat list; "pending" (approval policy) →
//  the waiting screen; invalid_invite_code (404) → inline error.
//

import Observation
import SwiftUI

@MainActor @Observable
final class JoinFamilyModel {
    var code = ""
    var isWorking = false
    var errorText: String?

    var canSubmit: Bool {
        !isWorking && !code.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

struct JoinFamilyView: View {
    @Environment(AppSession.self) private var session
    @State private var model = JoinFamilyModel()

    var body: some View {
        Form {
            Section {
                TextField("ABCD2345", text: Bindable(model).code)
                    .literalTextEntry(uppercased: true)
                    .font(.body.monospaced())
                    .onChange(of: model.code) { _, newValue in
                        let upper = newValue.uppercased()
                        if upper != newValue { model.code = upper }
                    }
            } header: {
                Text("Invite code")
            } footer: {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Any family member can read the code to you; the owner finds it in Settings.")
                    if let error = model.errorText {
                        Label(error, systemImage: "xmark.circle")
                            .foregroundStyle(.red)
                    }
                }
            }

            Section {
                Button {
                    submit()
                } label: {
                    if model.isWorking {
                        HStack {
                            ProgressView()
                            Text("Joining…")
                        }
                    } else {
                        Text("Join")
                    }
                }
                .disabled(!model.canSubmit)
            }
        }
        .navigationTitle("Join a Family")
    }

    private func submit() {
        model.errorText = nil
        model.isWorking = true
        let code = model.code.trimmingCharacters(in: .whitespaces)
        Task {
            defer { model.isWorking = false }
            do {
                // Routing (active / pendingApproval) happens inside
                // AppSession.join; nothing to do on success here.
                try await session.join(code: code)
            } catch APIError.notFound(let errorCode) {
                model.errorText = errorCode == "invalid_invite_code"
                    ? String(localized: "That code doesn't match any family. Check it and try again.")
                    : String(localized: "That code doesn't work anymore.")
            } catch APIError.conflict(let errorCode, let message) {
                switch errorCode {
                case "already_in_family": model.errorText = String(localized: "You're already in a family.")
                case "join_request_pending": model.errorText = String(localized: "You already have a pending request.")
                default: model.errorText = message ?? String(localized: "The server rejected that code.")
                }
            } catch {
                model.errorText = String(localized: "Can't reach the server. Try again.")
            }
        }
    }
}
