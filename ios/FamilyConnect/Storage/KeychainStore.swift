//
//  KeychainStore.swift
//  FamilyConnect
//
//  Thin wrapper over Keychain Services for the one secret this app holds:
//  the opaque session token. Follows exchange-ios's KeychainStore, minus
//  the iCloud-syncing mode — a *per-device* session token (protocol.md:
//  "opaque per-device session tokens") must never ride iCloud Keychain to
//  another device, so everything here is pinned deviceOnly:
//  `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, no synchronizable
//  bit — skipped by iCloud Keychain sync, encrypted backup restore to a
//  new device, and device-to-device migration alike.
//
//  Write strategy is update-then-add: SecItemUpdate first (the common
//  case after the first login), falling back to SecItemAdd on
//  errSecItemNotFound.
//

import Foundation
import Security

nonisolated enum KeychainError: Error, Equatable {
    case unhandled(OSStatus)
    case unexpectedData
}

nonisolated enum KeychainStore {
    /// Service identifier for every item this app writes. The bundle id,
    /// so a future App Group could be added without renaming items.
    static let service = Bundle.main.bundleIdentifier ?? "me.nettrash.FamilyConnect"

    /// Account under which the session token lives.
    static let tokenAccount = "session-token"

    // MARK: - Data

    /// Insert or update a blob under `account` (update-then-add).
    static func set(_ data: Data, account: String) throws {
        let baseQuery = query(account: account)
        let updateStatus = SecItemUpdate(
            baseQuery as CFDictionary,
            [kSecValueData as String: data] as CFDictionary
        )
        switch updateStatus {
        case errSecSuccess:
            return
        case errSecItemNotFound:
            var addQuery = baseQuery
            addQuery[kSecValueData as String] = data
            addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw KeychainError.unhandled(addStatus)
            }
        default:
            throw KeychainError.unhandled(updateStatus)
        }
    }

    /// The blob under `account`, or nil when absent.
    static func get(account: String) throws -> Data? {
        var fetchQuery = query(account: account)
        fetchQuery[kSecReturnData as String] = true
        fetchQuery[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(fetchQuery as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            guard let data = result as? Data else { throw KeychainError.unexpectedData }
            return data
        case errSecItemNotFound:
            return nil
        default:
            throw KeychainError.unhandled(status)
        }
    }

    /// Remove the item if present. No-op if absent.
    static func delete(account: String) throws {
        #if DEBUG
        if account == tokenAccount, injectedToken != nil { return }
        #endif
        let status = SecItemDelete(query(account: account) as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unhandled(status)
        }
    }

    // MARK: - String convenience (the token is UTF-8 text)

    static func setString(_ value: String, account: String) throws {
        #if DEBUG
        // Nothing to store when the token came from the command line: see
        // `getString`. Writing it would raise the very prompt that hook
        // exists to avoid.
        if account == tokenAccount, injectedToken != nil { return }
        #endif
        try set(Data(value.utf8), account: account)
    }

    static func getString(account: String) throws -> String? {
        #if DEBUG
        // THE MAC SCREENSHOT CAPTURE'S SESSION, handed in on the command
        // line (`-v1.sessionToken`) instead of stored here. DEBUG-only, and
        // the same argument shape as `-v1.serverURL`, `-v1.storeURL` and
        // `-v1.showScreen`.
        //
        // WHY IT EXISTS, measured 2026-09-17 with `sample`: an UNSIGNED
        // local build reading a real Keychain item raises the ACL
        // confirmation dialog, and every rebuild is a new binary, so
        // "Always Allow" does not carry over. On a Mac where the capture is
        // relaunching the app, that dialog can open on another Space —
        // invisible — and `SecItemCopyMatching` then blocks inside
        // `applicationDidFinishLaunching`, so the app comes up with NO
        // WINDOW AT ALL and nothing anywhere says why. Handing the token in
        // skips the Keychain, and with it the prompt.
        if account == tokenAccount, let injected = injectedToken { return injected }
        #endif
        guard let data = try get(account: account) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    #if DEBUG
    /// A session token passed on the command line, or nil. See `getString`.
    static var injectedToken: String? {
        guard let token = UserDefaults.standard.string(forKey: "v1.sessionToken"),
              !token.isEmpty else { return nil }
        return token
    }
    #endif

    // MARK: - Internals

    private static func query(account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}
