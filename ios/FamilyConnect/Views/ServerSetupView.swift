//
//  ServerSetupView.swift
//  FamilyConnect
//
//  First-run screen: point the app at the family's server. The text
//  field accepts what people actually type ("chat.example.com",
//  "192.168.1.10:8080/") and the normalizer makes it a proper base URL:
//  https:// is prepended when no scheme was given, the trailing slash is
//  stripped. Plain http:// is accepted ONLY for hosts the ATS
//  local-networking exception actually covers (private-range IPs,
//  localhost, *.local) — anywhere else http would just fail at request
//  time with an opaque ATS error, so we refuse it here with a clear
//  message instead. A warning still shows for the allowed-http case.
//
//  "Connect" probes GET /me (expects 401 + the documented error body) so
//  a typo'd host or a random web server is caught before the user ever
//  reaches the login screen.
//

import Observation
import SwiftUI

/// Pure URL normalization + policy, split from the view for clarity.

/// Per-screen state: what the user typed, what we think of it, and
/// whether a probe is in flight.
@MainActor @Observable
final class ServerSetupModel {
    var urlText = ""
    var isProbing = false
    var errorText: String?

    /// Live insecure-http advisory (recomputed as the user types).
    var insecureWarning: String? {
        if case .okInsecureLocal = ServerURLNormalizer.normalize(urlText) {
            return "This address uses plain http. That's fine for a server on your home network, but anyone on the same network can read the traffic."
        }
        return nil
    }
}

struct ServerSetupView: View {
    @Environment(AppSession.self) private var session
    @State private var model = ServerSetupModel()
    @FocusState private var fieldFocused: Bool

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("chat.example.com", text: Bindable(model).urlText)
                        #if os(iOS)
                        .keyboardType(.URL)
                        #endif
                        #if os(iOS)
                        .textContentType(.URL)
                        #endif
                        .literalTextEntry()
                        .focused($fieldFocused)
                        .onSubmit { connect() }
                } header: {
                    Text("Server address")
                } footer: {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Ask the family member who runs the server for its address.")
                        if let warning = model.insecureWarning {
                            Label(warning, systemImage: "exclamationmark.shield")
                                .foregroundStyle(.orange)
                        }
                        if let error = model.errorText {
                            Label(error, systemImage: "xmark.circle")
                                .foregroundStyle(.red)
                        }
                    }
                }

                Section {
                    Button {
                        connect()
                    } label: {
                        if model.isProbing {
                            HStack {
                                ProgressView()
                                Text("Checking…")
                            }
                        } else {
                            Text("Connect")
                        }
                    }
                    .disabled(model.isProbing || model.urlText.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .navigationTitle("Family Connect")
            .onAppear {
                // Prefill when the user came back via "change server".
                if model.urlText.isEmpty, let existing = AppSettings.serverURL {
                    model.urlText = existing.absoluteString
                }
            }
        }
    }

    private func connect() {
        model.errorText = nil
        let url: URL
        switch ServerURLNormalizer.normalize(model.urlText) {
        case .ok(let normalized), .okInsecureLocal(let normalized):
            url = normalized
        case .invalid(let reason):
            model.errorText = reason
            return
        }
        model.isProbing = true
        fieldFocused = false
        Task {
            defer { model.isProbing = false }
            do {
                try await session.setServer(url)
            } catch {
                model.errorText = "No Family Connect server answered at \(url.absoluteString). Check the address and your network."
            }
        }
    }
}
