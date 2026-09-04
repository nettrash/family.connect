//
//  CreateFamilyView.swift
//  FamilyConnect
//
//  Name the family, become its owner. The server creates the family chat
//  automatically (protocol.md POST /families), so success lands straight
//  in the active phase with a chat list that already has one row.
//

import Observation
import SwiftUI

@MainActor @Observable
final class CreateFamilyModel {
    var name = ""
    var isWorking = false
    var errorText: String?

    var canSubmit: Bool {
        !isWorking && !name.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

struct CreateFamilyView: View {
    @Environment(AppSession.self) private var session
    @State private var model = CreateFamilyModel()
    @FocusState private var fieldFocused: Bool

    var body: some View {
        Form {
            Section {
                TextField("The Smiths", text: Bindable(model).name)
                    #if os(iOS)
                    .textInputAutocapitalization(.words)
                    #endif
                    .focused($fieldFocused)
                    .submitLabel(.go)
                    .onSubmit { if model.canSubmit { submit() } }
            } header: {
                Text("Family name")
            } footer: {
                VStack(alignment: .leading, spacing: 8) {
                    Text("This names your family chat too. 1–64 characters.")
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
                            Text("Creating…")
                        }
                    } else {
                        Text("Create Family")
                    }
                }
                .disabled(!model.canSubmit)
            }
        }
        .navigationTitle("Create a Family")
        .onAppear {
            // One field: on the Mac it is already the place to type. iOS
            // waits for the tap (see AuthView.focusFirstField).
            #if os(macOS)
            fieldFocused = true
            #endif
        }
    }

    private func submit() {
        model.errorText = nil
        model.isWorking = true
        let name = model.name.trimmingCharacters(in: .whitespaces)
        Task {
            defer { model.isWorking = false }
            do {
                try await session.createFamily(name: name)
            } catch APIError.forbidden(let code) where code == "family_registration_disabled" {
                // Reachable only on a server that closed its door after
                // this screen was pushed — the gate itself hides the way
                // in (docs/protocol.md, "Starting a family").
                model.errorText = String(localized: "This server doesn't take new families.")
            } catch APIError.conflict(let code, let message) {
                model.errorText = code == "already_in_family"
                    ? String(localized: "You're already in a family. Pull to refresh.")
                    : (message ?? String(localized: "The server rejected that name."))
            } catch {
                model.errorText = String(localized: "Can't reach the server. Try again.")
            }
        }
    }
}
