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
    /// The subscription screen, between the name and the create, on a
    /// server that charges for a new family (docs/protocol.md, "Paid
    /// families").
    var showsSubscription = false

    var canSubmit: Bool {
        !isWorking && !name.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

struct CreateFamilyView: View {
    @Environment(AppSession.self) private var session
    @State private var model = CreateFamilyModel()
    @FocusState private var fieldFocused: Bool
    /// "New Family…" from inside the app — an account already in a family
    /// starting ANOTHER one (docs/protocol.md, "Several families"): the same
    /// field and button, with a line saying what owning it means.
    var anotherFamily = false
    /// Called once the family exists; the sheet that presented this closes.
    var onFinished: (() -> Void)?

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
                    if anotherFamily {
                        Text("Start your own family. You'll be its owner and can invite members of your other families.")
                    }
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
        // The name first — validated as it always was — then the products,
        // then the create: a family is never started for somebody the
        // server would refuse, and nobody pays before choosing a name.
        .sheet(isPresented: Bindable(model).showsSubscription) {
            SubscriptionSheet { create() }
        }
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
        // A server that charges, and no active subscription: sell it first.
        // With one, nothing is shown — the create goes straight through.
        if session.needsSubscriptionToCreate {
            model.showsSubscription = true
            return
        }
        create()
    }

    private func create() {
        model.errorText = nil
        model.isWorking = true
        let name = model.name.trimmingCharacters(in: .whitespaces)
        Task {
            defer { model.isWorking = false }
            do {
                try await session.createFamily(name: name)
                onFinished?()
            } catch let error where FamilyRules.failureMessage(
                error, maxFamilies: session.maxFamiliesPerAccount) != nil {
                // The several-families refusals: the cap, "you already own
                // one", and a server that keeps this door shut.
                model.errorText = FamilyRules.failureMessage(
                    error, maxFamilies: session.maxFamiliesPerAccount)
            } catch let error where PaidFamilies.failureMessage(error) != nil {
                // `subscription_required`: the subscription lapsed between
                // the screen and the create. Terminal — said, not retried.
                model.errorText = PaidFamilies.failureMessage(error)
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
