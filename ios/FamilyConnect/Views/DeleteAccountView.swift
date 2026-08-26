//
//  DeleteAccountView.swift
//  FamilyConnect
//
//  Deleting your own account, from inside the app, without asking anybody
//  — which is what App Store guideline 5.1.1(v) requires and what a
//  self-hosted family server should offer regardless (docs/protocol.md,
//  "Deleting an account").
//
//  SHARED between iOS and the Mac, like ChangePasswordView next door: it
//  is a short modal errand — read this, type your password, confirm — and
//  writing it twice would buy nothing. The Mac presents it with a width,
//  the phone as a full sheet.
//
//  The copy is the feature. What the server actually does is asymmetric —
//  the person is erased, the words stay — and somebody about to do
//  something irreversible is owed the whole of it before they type a
//  password, not a "this cannot be undone" and a shrug. Every line below
//  states one thing the server really does, in the order it matters:
//  what goes, what goes for somebody else too, what stays, and what
//  happens to the family.
//
//  Android counterpart: the delete-account flow in ui/settings/.
//

import os
import SwiftUI

struct DeleteAccountView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss

    @State private var password = ""
    @State private var isDeleting = false
    @State private var confirming = false
    /// Already localized when it is ASSIGNED, not when it is drawn:
    /// `Label(_:systemImage:)` takes a `String` through its
    /// `StringProtocol` overload, which performs no catalogue lookup at
    /// all, so a bare literal here would ship English to all eight
    /// languages. Every site that writes this wraps in `String(localized:)`.
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Form {
                consequencesSection
                passwordSection
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "exclamationmark.circle")
                            .font(.callout)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Delete Account")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isDeleting)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Delete", role: .destructive) { confirming = true }
                        .disabled(isDeleting || password.isEmpty)
                }
            }
            .overlay {
                if isDeleting { ProgressView() }
            }
            .confirmationDialog(
                "Delete your account?",
                isPresented: $confirming,
                titleVisibility: .visible
            ) {
                Button("Delete Account", role: .destructive) { delete() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This happens immediately and cannot be undone.")
            }
        }
        .interactiveDismissDisabled(isDeleting)
    }

    // MARK: - Sections

    /// Every line here is something the server does — see the file header.
    private var consequencesSection: some View {
        Section {
            Label(
                "Your account, password, profile picture and birthday are deleted, and every device you are signed in on is signed out.",
                systemImage: "person.crop.circle.badge.xmark")
            Label(
                "Your direct chats are deleted — for the other person too. So is your private chat with the assistant.",
                systemImage: "bubble.left.and.bubble.right")
            Label(
                "Your messages in the family chat, your board notes and your reactions stay. They are shown from then on as “Deleted account”.",
                systemImage: "text.bubble")
            if session.isOwner {
                Label(
                    "You own this family: ownership passes to the longest-standing remaining member. If you are its last member, the family is deleted with you — its chat, its board and its invite code.",
                    systemImage: "house")
            }
        } header: {
            Text("What happens")
        } footer: {
            Text("There is no grace period and no way to cancel afterwards.")
        }
    }

    private var passwordSection: some View {
        Section {
            SecureField("Password", text: $password)
                .textContentType(.password)
        } footer: {
            // The same reason POST /me/password asks for it: a live
            // session is not proof, and an unattended unlocked phone is
            // exactly what this protects against.
            Text("Type your password to confirm it is you. Being signed in is not proof.")
        }
    }

    // MARK: - Action

    private func delete() {
        errorText = nil
        isDeleting = true
        Task {
            do {
                try await session.deleteAccount(password: password)
                // Nothing to dismiss: the session is now at the sign-in
                // screen, and this whole subtree went with it.
            } catch APIError.unauthorized {
                // A WRONG PASSWORD, not a dead session — the server
                // answers 401 invalid_credentials here, and signing the
                // user out over a typo would be its own small disaster.
                isDeleting = false
                errorText = String(localized: "That password is not right.")
            } catch APIError.conflict(let code, _) where code == "validation" {
                isDeleting = false
                errorText = String(localized: "Type your password to confirm.")
            } catch {
                AppLog.api.error("Account deletion failed: \(String(describing: error))")
                isDeleting = false
                errorText = String(localized: "Couldn't delete your account. Try again.")
            }
        }
    }
}
