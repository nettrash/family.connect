//
//  KeychainStoreTests.swift
//  FamilyConnectTests
//
//  Round-trip against the real simulator keychain. Each test uses a
//  unique account so parallel runs never collide, and cleans up after
//  itself.
//
//  AN UNSIGNED TEST HOST CANNOT REACH THE KEYCHAIN AT ALL, and these
//  therefore SKIP rather than pass. `CODE_SIGNING_ALLOWED=NO` — what
//  `.github/workflows/ci.yml` passes, and what the workspace CLAUDE.md
//  recipe used to recommend for everything — produces a build with no
//  `application-identifier` entitlement, and the keychain needs one to
//  decide who owns an item, so every write fails with OSStatus -34018
//  (`errSecMissingEntitlement`).
//
//  They used to `return` early instead, which counts as a PASS. That is
//  the failure mode issue #20 was filed about, one level down: in CI the
//  executed-test count was right and the coverage was zero, and nothing
//  could see it. A skip is visible — it shows in the run's `skipped`
//  count, which the CI guard prints (issue #45).
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("KeychainStore")
struct KeychainStoreTests {

    /// True when this environment can write the keychain at all.
    ///
    /// Sampled ONCE per process, which is safe here in a way it was not
    /// for the location tests (#33): whether the host is signed cannot
    /// change mid-run, and unlike an authorization prompt this probe does
    /// not alter the thing it measures — it writes and deletes its own
    /// unique account.
    static let keychainWritable: Bool = {
        let probe = "probe-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: probe) }
        return (try? KeychainStore.setString("x", account: probe)) != nil
    }()

    @Test("set → get → delete round-trips a string",
          .enabled(if: KeychainStoreTests.keychainWritable))
    func roundTrip() throws {
        let account = "test-token-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: account) }

        try KeychainStore.setString("opaque-session-token", account: account)
        #expect(try KeychainStore.getString(account: account) == "opaque-session-token")

        try KeychainStore.delete(account: account)
        #expect(try KeychainStore.getString(account: account) == nil)
    }

    @Test("set overwrites in place (update-then-add path)",
          .enabled(if: KeychainStoreTests.keychainWritable))
    func overwrite() throws {
        let account = "test-overwrite-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: account) }

        try KeychainStore.setString("first", account: account)
        try KeychainStore.setString("second", account: account)

        #expect(try KeychainStore.getString(account: account) == "second")
    }

    @Test("get of a missing account is nil, delete of one is a no-op",
          .enabled(if: KeychainStoreTests.keychainWritable))
    func missingAccount() throws {
        let account = "test-missing-\(UUID().uuidString)"
        #expect(try KeychainStore.getString(account: account) == nil)
        try KeychainStore.delete(account: account) // must not throw
    }
}
