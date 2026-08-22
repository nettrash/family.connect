//
//  AuthView.swift
//  FamilyConnect
//
//  Login/Register on one screen behind a segmented picker — a family app
//  has exactly two entry stories ("I have an account here" / "I'm new")
//  and a segmented switch keeps both visible instead of hiding one behind
//  a link. Errors surface inline under the fields, mapped from the
//  server's stable machine codes (protocol.md §Error shape) so the copy
//  can be specific: `username_taken` → "That username is taken",
//  401 on login → wrong credentials, `validation` → the server's own
//  message (it names the offending rule).
//

import Observation
import SwiftUI

@MainActor @Observable
final class AuthModel {
    enum Mode: String, CaseIterable, Identifiable {
        case login = "Log In"
        case register = "Register"
        var id: String { rawValue }
    }

    var mode: Mode = .login
    var username = ""
    var displayName = ""
    var password = ""
    var isWorking = false
    var errorText: String?

    var canSubmit: Bool {
        guard !isWorking else { return false }
        let user = username.trimmingCharacters(in: .whitespaces)
        switch mode {
        case .login:
            return !user.isEmpty && !password.isEmpty
        case .register:
            // Server rules: username 3–32 [a-zA-Z0-9_.], password ≥ 8,
            // display name 1–64. Enforced server-side; we only gate on
            // non-emptiness so the server's message stays authoritative.
            return !user.isEmpty && !password.isEmpty
                && !displayName.trimmingCharacters(in: .whitespaces).isEmpty
        }
    }
}

struct AuthView: View {
    @Environment(AppSession.self) private var session
    @State private var model = AuthModel()

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Mode", selection: Bindable(model).mode) {
                        ForEach(AuthModel.Mode.allCases) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Color.clear)
                }

                Section {
                    TextField("Username", text: Bindable(model).username)
                        .textContentType(.username)
                        .literalTextEntry()
                    if model.mode == .register {
                        TextField("Display name", text: Bindable(model).displayName)
                            .textContentType(.name)
                    }
                    SecureField("Password", text: Bindable(model).password)
                        .textContentType(model.mode == .register ? .newPassword : .password)
                } footer: {
                    if let error = model.errorText {
                        Label(error, systemImage: "xmark.circle")
                            .foregroundStyle(.red)
                    } else if model.mode == .register {
                        Text("Usernames are 3–32 letters, digits, dots or underscores. Passwords need at least 8 characters.")
                    }
                }

                Section {
                    Button {
                        submit()
                    } label: {
                        if model.isWorking {
                            HStack {
                                ProgressView()
                                Text(model.mode == .login ? "Logging in…" : "Creating account…")
                            }
                        } else {
                            Text(model.mode == .login ? "Log In" : "Create Account")
                        }
                    }
                    .disabled(!model.canSubmit)
                }

                Section {
                    Button("Change server…") {
                        session.requestServerChange()
                    }
                    .font(.footnote)
                } footer: {
                    if let server = AppSettings.serverURL {
                        Text("Connected to \(server.absoluteString)")
                    }
                }
            }
            .navigationTitle(model.mode == .login ? "Welcome Back" : "Join the Family")
        }
    }

    private func submit() {
        model.errorText = nil
        model.isWorking = true
        let username = model.username.trimmingCharacters(in: .whitespaces)
        let displayName = model.displayName.trimmingCharacters(in: .whitespaces)
        let password = model.password
        let isLogin = model.mode == .login
        Task {
            defer { model.isWorking = false }
            do {
                if isLogin {
                    try await session.login(username: username, password: password)
                } else {
                    try await session.register(username: username, displayName: displayName, password: password)
                }
            } catch {
                model.errorText = Self.describe(error, isLogin: isLogin)
            }
        }
    }

    /// Map API errors to inline copy, keyed on the machine codes.
    static func describe(_ error: Error, isLogin: Bool) -> String {
        switch error {
        case APIError.unauthorized:
            return isLogin
                ? "Wrong username or password."
                : "The server rejected the request. Try again."
        case APIError.conflict(let code, let message):
            if code == "username_taken" { return "That username is taken." }
            return message ?? "The server rejected the request."
        case APIError.transport:
            return "Can't reach the server. Check your connection."
        case APIError.server:
            return "The server had a problem. Try again in a moment."
        default:
            return "Something went wrong. Try again."
        }
    }
}
