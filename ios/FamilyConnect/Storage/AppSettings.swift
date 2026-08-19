//
//  AppSettings.swift
//  FamilyConnect
//
//  Typed accessors over the app's UserDefaults. Keys are versioned
//  ("v1.…") so a future schema change can migrate by key prefix instead
//  of guessing what an unversioned value meant. Nothing sensitive lives
//  here — the session token is in the Keychain (KeychainStore); this is
//  routing/bookkeeping state only.
//
//  Declared in PrivacyInfo.xcprivacy under accessed-API reason CA92.1
//  (app accesses only its own defaults).
//

import Foundation

nonisolated enum AppSettings {

    private static var defaults: UserDefaults { .standard }

    private enum Key {
        static let serverURL = "v1.serverURL"
        static let currentUserID = "v1.currentUserID"
        static let joinPending = "v1.joinPending"
        static let deviceRegistered = "v1.deviceRegistered"
    }

    /// The user-entered server base URL (normalized: scheme present,
    /// no trailing slash). nil until the setup screen confirms one.
    static var serverURL: URL? {
        get { defaults.string(forKey: Key.serverURL).flatMap(URL.init(string:)) }
        set {
            if let newValue {
                defaults.set(newValue.absoluteString, forKey: Key.serverURL)
            } else {
                defaults.removeObject(forKey: Key.serverURL)
            }
        }
    }

    /// The signed-in user's server id; lets the sync layer attribute
    /// "mine" without a /me round-trip.
    static var currentUserID: Int64? {
        get {
            let value = defaults.object(forKey: Key.currentUserID) as? NSNumber
            return value?.int64Value
        }
        set {
            if let newValue {
                defaults.set(NSNumber(value: newValue), forKey: Key.currentUserID)
            } else {
                defaults.removeObject(forKey: Key.currentUserID)
            }
        }
    }

    /// True while a join request is outstanding. Persisted so a relaunch
    /// can still detect "was waiting, now neither family nor request ⇒
    /// declined" (the protocol's only rejection signal).
    static var joinPending: Bool {
        get { defaults.bool(forKey: Key.joinPending) }
        set { defaults.set(newValue, forKey: Key.joinPending) }
    }

    /// True once POST /devices succeeded for this install, so the device
    /// row is registered exactly once and not on every launch.
    static var deviceRegistered: Bool {
        get { defaults.bool(forKey: Key.deviceRegistered) }
        set { defaults.set(newValue, forKey: Key.deviceRegistered) }
    }

    /// Remove everything this type owns; `keepServerURL` preserves the
    /// server row (logout keeps it; server change does not).
    static func wipe(keepServerURL: Bool) {
        if !keepServerURL { defaults.removeObject(forKey: Key.serverURL) }
        defaults.removeObject(forKey: Key.currentUserID)
        defaults.removeObject(forKey: Key.joinPending)
        defaults.removeObject(forKey: Key.deviceRegistered)
    }
}
