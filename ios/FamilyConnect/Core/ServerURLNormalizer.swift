//
//  ServerURLNormalizer.swift
//  FamilyConnect
//
//  Turning what somebody typed into a server URL the app can hold.
//
//  Lives in Core rather than beside the setup screen it was written for:
//  AppSettings needs it to normalise the URL compiled into a build, and
//  the Mac app needs it for a setup screen of its own. It is a pure
//  parser with no view in it — the only reason it was ever in a view file
//  is that the setup screen was the first thing to need it.
//

import Foundation

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
