//
//  MacScreenshotRoute.swift
//  FamilyConnect (macOS)
//
//  WHICH SCREEN TO OPEN AT LAUNCH, for the Mac App Store capture
//  (`ios/scripts/capture-mac-screenshot.sh`). DEBUG-only, and read the same
//  way as `-v1.serverURL` and `-v1.storeURL`: a launch argument in the
//  NSArgumentDomain, never a stored setting.
//
//  WHY THIS EXISTS. The iPhone set is shot by StoreScreenshotUITests, which
//  taps its way through the app. On macOS there is no such runner here, so
//  the capture script has always asked a person to navigate and only
//  automated the shutter. Doing that from a terminal is not possible at all:
//  clicking or typing into another app needs Accessibility permission
//  (System Events fails with -25211 without it), and macOS 14 and later
//  refuse even cross-app ACTIVATION from a background process. Measured
//  2026-09-17.
//
//  What the same measurements showed is that `screencapture -l <window id>`
//  photographs a window that is not on the active Space at all, in colour,
//  with live traffic lights. So the only thing the capture could not do by
//  itself was reach the screens. One launch argument per screen closes that:
//  the script relaunches the app once per shot, and the whole Mac set comes
//  out unattended, the way the iPhone set does.
//
//  It changes nothing in a shipped build: `DEBUG` is defined only by the
//  Debug configuration, and a Release build has no `-v1.showScreen`.
//

#if DEBUG && os(macOS)
import Foundation

enum MacScreenshotRoute: String {
    /// The family sheet — members, invite code, join requests, Reports.
    case family
    /// The board window.
    case board
    /// The Settings window (1.1 made it a window; it was a sheet in 1.0).
    case settings
    /// The newest answered message's chain, as a sheet over the chat.
    case thread
    /// Every poll still open, as a sheet over the chat.
    case polls

    static var requested: MacScreenshotRoute? {
        guard let raw = UserDefaults.standard.string(forKey: "v1.showScreen"),
              !raw.isEmpty else { return nil }
        return MacScreenshotRoute(rawValue: raw)
    }
}
#endif
