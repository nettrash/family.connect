//
//  AppVersion.swift
//  FamilyConnect
//
//  What build this is, for the bottom of Settings.
//
//  Worth having on screen rather than only in the store listing: this is a
//  self-hosted app whose server the family runs themselves, so "which
//  version are you on?" is a question that actually gets asked when a
//  device is behaving differently from the others.
//
//  Marketing version and build number, exactly as Android already showed
//  them — `1.0 (51)`. Both come from the bundle, so nothing has to be kept
//  in step by hand.
//

import Foundation

enum AppVersion {

    /// `CFBundleShortVersionString` — the one people say out loud ("1.0").
    static var short: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
    }

    /// `CFBundleVersion` — the build, which is what actually distinguishes
    /// two devices claiming the same version.
    static var build: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "—"
    }

    /// `1.0 (51)`.
    static var display: String { "\(short) (\(build))" }

    /// The line Settings shows: the app's display name and the build.
    ///
    /// Deliberately NOT localized — a product name and two numbers have
    /// nothing to translate, and the name is the same in every language.
    static var settingsLine: String {
        let name = Bundle.main.infoDictionary?["CFBundleDisplayName"] as? String
            ?? Bundle.main.infoDictionary?["CFBundleName"] as? String
            ?? "Family"
        return "\(name) \(display)"
    }
}
