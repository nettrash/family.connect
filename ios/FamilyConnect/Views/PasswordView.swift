//
//  PasswordView.swift
//  FamilyConnect
//
//  Two sheets over the same idea: setting a password.
//
//  ChangePasswordView is for your own account and asks for the current one
//  — a live session is not proof of knowing it, and the case that matters
//  is an unattended unlocked phone (protocol.md, "Auth").
//
//  ResetPasswordView is the owner's, for a member who has forgotten
//  theirs, and asks for no current password at all. It says out loud what
//  it will do, because it signs that member out of every device they have.
//
//  Both confirm the new password twice locally: the server has no way to
//  tell a typo from an intention, and the cost of getting it wrong is
//  being locked out of a self-hosted server with no reset email.
//
//  Android counterpart: ui/settings/PasswordScreen.kt
//

import SwiftUI

/// Shared shape: two matching entries, a length rule, one call, one error.
private struct PasswordFields: View {
    let newPasswordLabel: String
    @Binding var newPassword: String
    @Binding var confirmation: String

    var body: some View {
        SecureField(newPasswordLabel, text: $newPassword)
            .textContentType(.newPassword)
        SecureField("Confirm New Password", text: $confirmation)
            .textContentType(.newPassword)
    }
}

/// The rule the server enforces, checked here too so the sheet can say so
/// before spending a round trip.
private enum PasswordRules {
    static let minimumLength = 8

    static func problem(new: String, confirmation: String) -> String? {
        if new.count < minimumLength {
            return "Use at least \(minimumLength) characters."
        }
        if new != confirmation {
            return "Those two do not match."
        }
        return nil
    }
}

struct ChangePasswordView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var current = ""
    @State private var newPassword = ""
    @State private var confirmation = ""
    @State private var isSaving = false
    @State private var errorText: String?
    @State private var didSucceed = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    SecureField("Current Password", text: $current)
                        .textContentType(.password)
                } footer: {
                    Text("Your other devices will be signed out. This one stays signed in.")
                }
                Section {
                    PasswordFields(
                        newPasswordLabel: "New Password",
                        newPassword: $newPassword,
                        confirmation: $confirmation)
                }
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "exclamationmark.circle")
                            .font(.callout)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Change Password")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }
                        .disabled(isSaving || current.isEmpty || newPassword.isEmpty)
                }
            }
            .overlay {
                if isSaving { ProgressView() }
            }
            .alert("Password changed", isPresented: $didSucceed) {
                Button("OK") { dismiss() }
            } message: {
                Text("Your other devices have been signed out.")
            }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private func save() {
        if let problem = PasswordRules.problem(new: newPassword, confirmation: confirmation) {
            errorText = problem
            return
        }
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                try await coordinator.api.changePassword(current: current, new: newPassword)
                didSucceed = true
            } catch APIError.unauthorized {
                // The server answers 401 for a WRONG CURRENT PASSWORD here,
                // which is not a dead session — treating it as one would
                // sign the user out for a typo.
                errorText = "That current password is not right."
            } catch {
                errorText = "Couldn't change your password. Try again."
            }
        }
    }
}

/// The owner setting a member's password for them.
struct ResetPasswordView: View {
    let member: MemberDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var newPassword = ""
    @State private var confirmation = ""
    @State private var isSaving = false
    @State private var errorText: String?
    @State private var didSucceed = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    PasswordFields(
                        newPasswordLabel: "New Password",
                        newPassword: $newPassword,
                        confirmation: $confirmation)
                } header: {
                    Text("New password for \(member.displayName)")
                } footer: {
                    // Said plainly, because it is not obvious and it is not
                    // undoable: the member is signed out everywhere.
                    Text("""
                        \(member.displayName) will be signed out on every device and will need \
                        this password to sign back in. Tell it to them somewhere safe — the \
                        server has no way to email it.
                        """)
                }
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "exclamationmark.circle")
                            .font(.callout)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Reset Password")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Reset") { save() }
                        .disabled(isSaving || newPassword.isEmpty)
                }
            }
            .overlay {
                if isSaving { ProgressView() }
            }
            .alert("Password reset", isPresented: $didSucceed) {
                Button("OK") { dismiss() }
            } message: {
                Text("\(member.displayName) has been signed out everywhere.")
            }
        }
        .interactiveDismissDisabled(isSaving)
    }

    private func save() {
        if let problem = PasswordRules.problem(new: newPassword, confirmation: confirmation) {
            errorText = problem
            return
        }
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                try await coordinator.api.resetMemberPassword(
                    userID: member.id, newPassword: newPassword)
                didSucceed = true
            } catch {
                errorText = "Couldn't reset that password. Try again."
            }
        }
    }
}
