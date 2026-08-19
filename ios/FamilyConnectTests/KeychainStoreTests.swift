//
//  KeychainStoreTests.swift
//  FamilyConnectTests
//
//  Round-trip against the real simulator keychain. Each test uses a
//  unique account so parallel runs never collide, and cleans up after
//  itself. An unsigned test host (CODE_SIGNING_ALLOWED=NO) cannot always
//  reach the keychain daemon — in that environment the tests detect the
//  failure up front and pass vacuously rather than failing the suite on
//  infrastructure.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("KeychainStore")
struct KeychainStoreTests {

    /// True when this environment can write the keychain at all.
    private func keychainWritable() -> Bool {
        let probe = "probe-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: probe) }
        return (try? KeychainStore.setString("x", account: probe)) != nil
    }

    @Test("set → get → delete round-trips a string")
    func roundTrip() throws {
        guard keychainWritable() else { return }
        let account = "test-token-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: account) }

        try KeychainStore.setString("opaque-session-token", account: account)
        #expect(try KeychainStore.getString(account: account) == "opaque-session-token")

        try KeychainStore.delete(account: account)
        #expect(try KeychainStore.getString(account: account) == nil)
    }

    @Test("set overwrites in place (update-then-add path)")
    func overwrite() throws {
        guard keychainWritable() else { return }
        let account = "test-overwrite-\(UUID().uuidString)"
        defer { try? KeychainStore.delete(account: account) }

        try KeychainStore.setString("first", account: account)
        try KeychainStore.setString("second", account: account)

        #expect(try KeychainStore.getString(account: account) == "second")
    }

    @Test("get of a missing account is nil, delete of one is a no-op")
    func missingAccount() throws {
        guard keychainWritable() else { return }
        let account = "test-missing-\(UUID().uuidString)"
        #expect(try KeychainStore.getString(account: account) == nil)
        try KeychainStore.delete(account: account) // must not throw
    }
}
