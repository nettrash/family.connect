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
        guard !text.isEmpty else { return .invalid(reason: String(localized: "Enter the server address.")) }
        if !text.contains("://") {
            text = "https://" + text
        }
        while text.hasSuffix("/") { text.removeLast() }
        guard let components = URLComponents(string: text),
              let scheme = components.scheme?.lowercased(),
              let host = components.host, !host.isEmpty,
              let url = components.url else {
            return .invalid(reason: String(localized: "That doesn't look like a valid address."))
        }
        switch scheme {
        case "https":
            return .ok(url)
        case "http":
            if isLocalNetworkHost(host) {
                return .okInsecureLocal(url)
            }
            return .invalid(reason: String(localized: "Plain http only works for servers on your local network. Use https for \(host)."))
        default:
            return .invalid(reason: String(localized: "Use an https:// or http:// address."))
        }
    }

    /// Is this a host plain http may go to — i.e. one on the family's own
    /// network?
    ///
    /// The answer has to track Info.plist's `NSAllowsLocalNetworking`,
    /// because a URL this says yes to and ATS says no to fails later with
    /// a network error nobody can act on. Apple defines that key as
    /// controlling "whether App Transport Security (ATS) allows your app
    /// to connect to unqualified domains, `.local` domains, and IP
    /// addresses using IPv4 or IPv6" — three categories, and this
    /// function used to implement one and a half of them (issue #33):
    ///
    ///   * UNQUALIFIED names — `nas`, `raspberrypi`, `fileserver` — were
    ///     REFUSED, with a message telling the user to use https for a
    ///     host sitting on their own LAN. A single-label name resolved
    ///     over mDNS or a DHCP search domain is the ordinary way to reach
    ///     a home box, and this is a self-hosted app: that is not an edge
    ///     case, it is the intended path. Now accepted.
    ///   * `.local` was already accepted.
    ///   * IP literals: only dotted quads were understood, so every IPv6
    ///     literal — `http://[fd00::1]` off the home router, even
    ///     `http://[::1]` — fell through to "use https". Now accepted for
    ///     the local blocks.
    ///
    /// DELIBERATELY NARROWER THAN ATS in one place. ATS's exception waves
    /// through *any* IP literal, public ones included; this does not.
    /// `http://203.0.113.9` is a family's messages in clear text across
    /// the internet, and nothing about "self-hosted" survives that. Only
    /// blocks that cannot route off a home network are allowed: RFC1918,
    /// loopback, link-local, and the v6 equivalents.
    ///
    /// The Android twin (`ServerUrlNormalizer.kt`) validates through
    /// OkHttp's `HttpUrl` and accepts every host, warning about cleartext
    /// in the UI instead of refusing it — so it already accepted `nas`
    /// and `[fd00::1]`. This closes the half of that gap that was costing
    /// iOS users a working setup; the other half (iOS still refusing
    /// public-address http where Android merely warns) is a deliberate
    /// platform difference, since ATS would block the connection anyway.
    static func isLocalNetworkHost(_ host: String) -> Bool {
        var name = host.lowercased()
        // URLComponents hands back an IPv6 literal WITH its brackets
        // ("[fd00::1]") and with the zone id percent-decoded
        // ("[fe80::1%en0]"). Strip both before reading an address out.
        if name.hasPrefix("["), name.hasSuffix("]") {
            let inner = name.dropFirst().dropLast()
            return isLocalIPv6(String(inner.prefix(while: { $0 != "%" })))
        }
        // A trailing dot is the DNS root, not another label — "nas." is
        // still one label, and still unqualified.
        if name.hasSuffix(".") { name.removeLast() }
        guard !name.isEmpty else { return false }
        if name.hasSuffix(".local") { return true }
        // Unqualified: no dot at all. `localhost` lands here too.
        if !name.contains(".") { return true }
        return isPrivateIPv4(name)
    }

    /// The v4 blocks that cannot leave a home network.
    private static func isPrivateIPv4(_ text: String) -> Bool {
        let labels = text.split(separator: ".", omittingEmptySubsequences: false)
        // Every label must be ASCII digits: `Int` would otherwise accept a
        // signed "+1", and a name is not an address just because the
        // pieces parse.
        guard labels.count == 4,
              labels.allSatisfy({ !$0.isEmpty && $0.allSatisfy { $0.isASCII && $0.isNumber } })
        else { return false }
        let octets = labels.compactMap { Int($0) }
        guard octets.count == 4, octets.allSatisfy({ (0...255).contains($0) }) else { return false }
        if octets[0] == 127 { return true }                                  // loopback 127/8
        if octets[0] == 10 { return true }                                   // 10/8
        if octets[0] == 172 && (16...31).contains(octets[1]) { return true }  // 172.16/12
        if octets[0] == 192 && octets[1] == 168 { return true }              // 192.168/16
        if octets[0] == 169 && octets[1] == 254 { return true }              // 169.254/16 link-local
        return false
    }

    /// The v6 blocks a home network hands out, read off the TEXT rather
    /// than through `inet_pton`: the only question is which prefix the
    /// address falls in, and every one of them is decided by the first
    /// hextet. Anything that does not parse is not local.
    private static func isLocalIPv6(_ text: String) -> Bool {
        // The two local forms that begin with the "::" compressor.
        if text.hasPrefix("::") {
            if text == "::1" { return true }                                 // loopback ::1/128
            // A v4 address wearing a v6 coat — judge it as v4.
            if text.hasPrefix("::ffff:") { return isPrivateIPv4(String(text.dropFirst(7))) }
            return false
        }
        guard let separator = text.firstIndex(of: ":"),
              let first = UInt16(text[text.startIndex..<separator], radix: 16)
        else { return false }
        if (0xfe80...0xfebf).contains(first) { return true }                 // fe80::/10 link-local
        if (0xfc00...0xfdff).contains(first) { return true }                 // fc00::/7 unique-local
        return false
    }
}
