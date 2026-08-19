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
nonisolated enum ServerURLNormalizer {

    enum Verdict: Equatable {
        case ok(URL)
        /// http, but to a local/private host ATS lets through — allowed
        /// with a warning.
        case okInsecureLocal(URL)
        case invalid(reason: String)
    }

    static func normalize(_ raw: String) -> Verdict {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return .invalid(reason: "Enter the server address.") }
        if !text.contains("://") {
            text = "https://" + text
        }
        while text.hasSuffix("/") { text.removeLast() }
        guard let components = URLComponents(string: text),
              let scheme = components.scheme?.lowercased(),
              let host = components.host, !host.isEmpty,
              let url = components.url else {
            return .invalid(reason: "That doesn't look like a valid address.")
        }
        switch scheme {
        case "https":
            return .ok(url)
        case "http":
            if isLocalNetworkHost(host) {
                return .okInsecureLocal(url)
            }
            return .invalid(reason: "Plain http only works for servers on your local network. Use https for \(host).")
        default:
            return .invalid(reason: "Use an https:// or http:// address.")
        }
    }

    /// Hosts covered by the ATS NSAllowsLocalNetworking exception: RFC1918
    /// private IPv4 ranges, loopback, link-local, and mDNS *.local names.
    static func isLocalNetworkHost(_ host: String) -> Bool {
        let lowered = host.lowercased()
        if lowered == "localhost" || lowered.hasSuffix(".local") { return true }
        let parts = lowered.split(separator: ".").compactMap { Int($0) }
        guard parts.count == 4, parts.allSatisfy({ (0...255).contains($0) }) else { return false }
        if parts[0] == 127 { return true }                               // loopback
        if parts[0] == 10 { return true }                                // 10/8
        if parts[0] == 172 && (16...31).contains(parts[1]) { return true } // 172.16/12
        if parts[0] == 192 && parts[1] == 168 { return true }            // 192.168/16
        if parts[0] == 169 && parts[1] == 254 { return true }            // link-local
        return false
    }
}

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
                        .keyboardType(.URL)
                        .textContentType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
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
