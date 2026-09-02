//
//  ServerURLNormalizerTests.swift
//  FamilyConnectTests
//
//  The normalizer is the only thing between "whatever the user typed" and
//  every request this app will ever make, and it had never been tested.
//
//  What these mostly pin is a POLICY, not a format: which addresses plain
//  http is allowed to reach. That policy has to track Info.plist's
//  `NSAllowsLocalNetworking` — Apple defines that key as letting ATS
//  connect to "unqualified domains, .local domains, and IP addresses using
//  IPv4 or IPv6" — because an address this accepts and ATS refuses fails
//  later as an unexplained network error, and an address this refuses and
//  ATS would have allowed is a family locked out of their own server.
//
//  The second kind was real (issue #33): `http://nas` and `http://[fd00::1]`
//  were both rejected with "use https for nas", advice that cannot be
//  followed on a box that has no certificate. The unqualified name is how
//  a LAN machine is normally reached, and this is a SELF-HOSTED app — that
//  is the intended path, not an edge case. Both are now accepted, with the
//  cleartext warning the setup screen already shows.
//
//  The suite is deliberately explicit about what must STILL be refused:
//  the fix widened a hole that leads straight to a family's messages in
//  clear text, and the boundary is the whole value of it.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Server URL normalizer")
struct ServerURLNormalizerTests {

    // MARK: - Reading a verdict

    /// The URL of an accepted verdict, https or local-http; nil when the
    /// input was refused. Reasons are user-facing localized sentences, so
    /// nothing here compares against their text.
    private func accepted(_ raw: String) -> URL? {
        switch ServerURLNormalizer.normalize(raw) {
        case .ok(let url), .okInsecureLocal(let url): return url
        case .invalid: return nil
        }
    }

    /// True only for `.okInsecureLocal` — accepted, and the screen shows
    /// the "anyone on this network can read it" warning.
    private func acceptedAsLocalHTTP(_ raw: String) -> Bool {
        if case .okInsecureLocal = ServerURLNormalizer.normalize(raw) { return true }
        return false
    }

    private func refused(_ raw: String) -> Bool {
        if case .invalid = ServerURLNormalizer.normalize(raw) { return true }
        return false
    }

    // MARK: - Canonical shape

    /// No scheme means https, never http: guessing http for a bare name
    /// would silently downgrade somebody who has TLS. The Android twin
    /// makes the same guess.
    @Test("A bare host becomes https")
    func bareHostDefaultsToHTTPS() {
        #expect(accepted("chat.example.com")?.absoluteString == "https://chat.example.com")
        #expect(accepted("  chat.example.com  ")?.absoluteString == "https://chat.example.com")
    }

    /// The `serverURL` invariant every caller relies on: no trailing
    /// slash, because paths are appended to this by string in places.
    @Test("Trailing slashes are stripped, however many")
    func trailingSlashesStripped() {
        #expect(accepted("https://chat.example.com/")?.absoluteString == "https://chat.example.com")
        #expect(accepted("https://chat.example.com///")?.absoluteString == "https://chat.example.com")
    }

    @Test("A port survives normalization")
    func portSurvives() {
        #expect(accepted("https://chat.example.com:8443")?.absoluteString
                == "https://chat.example.com:8443")
        #expect(accepted("http://192.168.1.10:8080")?.absoluteString
                == "http://192.168.1.10:8080")
    }

    @Test("https is accepted for any host, with no warning")
    func httpsIsAlwaysPlainOK() {
        for text in ["https://chat.example.com", "https://nas", "https://[2606:4700::1111]"] {
            guard case .ok = ServerURLNormalizer.normalize(text) else {
                Issue.record("\(text) should be plainly ok")
                continue
            }
        }
    }

    // MARK: - What is refused outright

    @Test("Nothing typed is refused, and so is whitespace")
    func emptyIsRefused() {
        #expect(refused(""))
        #expect(refused("   "))
        #expect(refused("\n\t "))
    }

    @Test("A scheme that is not http(s) is refused")
    func foreignSchemeRefused() {
        #expect(refused("ftp://nas"))
        #expect(refused("ws://nas"))
        #expect(refused("familyconnect://share"))
        #expect(refused("javascript://alert"))
    }

    @Test("A URL with no host at all is refused")
    func hostlessRefused() {
        #expect(refused("http://"))
        #expect(refused("https://"))
        #expect(refused("https:///path"))
    }

    // MARK: - The local-network exception (issue #33)

    /// The regression the issue names first. `nas`, `raspberrypi`,
    /// `fileserver` — an unqualified name resolved by mDNS or a DHCP
    /// search domain is how a home box is reached, ATS's local-networking
    /// exception covers it by Apple's own one-line definition, and this
    /// used to answer "use https for nas".
    @Test("Plain http to an unqualified LAN name is accepted with a warning")
    func unqualifiedNamesAreLocal() {
        for text in [
            "http://nas",
            "http://raspberrypi",
            "http://fileserver:8080",
            "http://NAS",            // case must not matter
            "http://nas.",           // a trailing dot is the DNS root, still one label
            "http://localhost",
            "http://localhost:8080",
        ] {
            #expect(acceptedAsLocalHTTP(text), "\(text) should be reachable over plain http")
        }
    }

    @Test("Plain http to a .local name is accepted with a warning")
    func bonjourNamesAreLocal() {
        #expect(acceptedAsLocalHTTP("http://homeserver.local"))
        #expect(acceptedAsLocalHTTP("http://Home-Server.LOCAL:8080"))
        #expect(acceptedAsLocalHTTP("http://nas.home.local"))
    }

    /// Every RFC1918 block, plus loopback and link-local, at both ends of
    /// the ranges that have ends.
    @Test("Plain http to a private IPv4 address is accepted with a warning")
    func privateIPv4IsLocal() {
        for text in [
            "http://10.0.0.5", "http://10.255.255.254",
            "http://172.16.0.1", "http://172.31.255.254",
            "http://192.168.1.10:8080",
            "http://127.0.0.1:3000",
            "http://169.254.10.1",
        ] {
            #expect(acceptedAsLocalHTTP(text), "\(text) should be reachable over plain http")
        }
    }

    /// The other half of the issue. URLComponents hands an IPv6 literal
    /// back WITH its brackets and with the zone id percent-decoded, and
    /// the old dotted-quad splitter understood neither — so a router's
    /// `fd00::/8` address, and even `::1`, were told to use https.
    @Test("Plain http to a local IPv6 literal is accepted with a warning")
    func localIPv6IsLocal() {
        for text in [
            "http://[::1]",                       // loopback
            "http://[::1]:8080",
            "http://[fd00::1]",                   // unique-local, what a home router hands out
            "http://[fd12:3456:789a::1]:8443",
            "http://[fc00::1]",                   // the bottom of fc00::/7
            "http://[fdff:ffff::1]",              // the top of it
            "http://[fe80::1]",                   // link-local
            "http://[fe80::1%25en0]",             // …with a zone id
            "http://[FE80::1]",                   // case must not matter
            "http://[::ffff:192.168.1.10]",       // a private v4 address in a v6 coat
        ] {
            #expect(acceptedAsLocalHTTP(text), "\(text) should be reachable over plain http")
        }
    }

    // MARK: - What plain http must STILL never reach

    /// The point of the whole function. Widening it must not turn a typo
    /// into a family's messages crossing the internet in clear text.
    @Test("Plain http to a public name is still refused")
    func publicNamesRefuseHTTP() {
        #expect(refused("http://chat.example.com"))
        #expect(refused("http://nas.example.com"))
        #expect(refused("http://example.co.uk:8080"))
        #expect(refused("http://xn--80ak6aa92e.com"))
    }

    /// Deliberately NARROWER than ATS, which waves through any IP
    /// literal: a public address over http is exactly the leak this app
    /// exists not to have.
    @Test("Plain http to a public IP literal is still refused")
    func publicAddressesRefuseHTTP() {
        for text in [
            "http://8.8.8.8",
            "http://203.0.113.9:8080",
            "http://[2606:4700::1111]",
            "http://[2001:db8::1]",
            "http://[::]",                        // the unspecified address is not a server
            "http://[fec0::1]",                   // site-local, deprecated by RFC 3879
        ] {
            #expect(refused(text), "\(text) must not be reachable over plain http")
        }
    }

    /// Just outside each private block — the arithmetic, not the idea.
    @Test("The private-IPv4 boundaries are exact")
    func ipv4BoundariesAreExact() {
        for text in [
            "http://11.0.0.1",            // 10/8 ends at 10.
            "http://9.255.255.255",
            "http://172.15.255.255",      // 172.16/12 is 172.16 … 172.31
            "http://172.32.0.1",
            "http://192.167.1.1",         // 192.168/16, and nothing either side
            "http://192.169.1.1",
            "http://169.253.1.1",         // 169.254/16 link-local only
            "http://169.255.1.1",
            "http://128.0.0.1",           // not 127/8
        ] {
            #expect(refused(text), "\(text) is not a private address")
        }
    }

    /// Something shaped like an address but not one is a NAME, and a
    /// dotted name is qualified — so it is public, and http is refused.
    @Test("Near-miss addresses are treated as public names, not as private ones")
    func malformedAddressesAreNotLocal() {
        for text in [
            "http://192.168.1",           // three octets is not an address
            "http://192.168.1.1.1",       // nor is five
            "http://192.168.1.999",       // out of range
            "http://192.168.+1.1",        // Int() would take the sign; a host has no sign
            "http://10.a.0.1",
        ] {
            #expect(refused(text), "\(text) must not pass as a private address")
        }
    }

    // MARK: - The compiled-in default server

    /// `AppSettings.defaultServerURL` runs the build setting through this
    /// same parser, so the widened rule has to hold for a store build too:
    /// a `Release-nettrash` pointed at a LAN box gets a URL rather than
    /// silently falling back to the setup screen.
    @Test("A LAN default server address normalizes rather than being dropped")
    func aLANDefaultWouldBeAdopted() {
        #expect(accepted("http://nas:8080")?.absoluteString == "http://nas:8080")
        #expect(accepted("http://[fd00::1]")?.absoluteString == "http://[fd00::1]")
    }
}
