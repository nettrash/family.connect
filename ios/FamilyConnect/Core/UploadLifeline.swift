//
//  UploadLifeline.swift
//  FamilyConnect
//
//  Keeps the app alive long enough to finish an attachment upload that was
//  already started when the person left.
//
//  WHY THIS EXISTS. `sendMedia` uploads every attachment and only then
//  enqueues the message row, which is deliberate — the row claims the whole
//  set at once, so a half-finished upload must leave nothing behind rather
//  than an empty bubble. The cost of that ordering is that a send in flight
//  exists ONLY as a running Task: nothing is persisted, so `sweepOutbox()`
//  has nothing to retry after a relaunch, unlike a text message. Suspend
//  the app mid-upload and the send is simply gone.
//
//  A background task buys the seconds an upload already in progress needs
//  to land, which covers the case that actually happens: someone presses
//  send and immediately switches apps.
//
//  BE CLEAR ABOUT WHAT THIS DOES NOT DO. If the allowance runs out first —
//  a large video on a slow connection — the send is lost exactly as before:
//  nothing is persisted, so there is nothing for `sweepOutbox()` to retry,
//  and the prepared files are orphaned in tmp with no owner. Making a media
//  send survive that needs the send represented in the model rather than in
//  a running Task, which is a larger design than this file. What is here
//  narrows the window; it does not close it.
//
//  iOS only. macOS does not suspend an app for being in the background, so
//  there is nothing to hold open; the no-op keeps `sendMedia` free of
//  platform conditionals.
//
//  Android counterpart: none needed — uploads there run in a coroutine on
//  an application-scoped repository, not tied to a screen.
//

import Foundation
import os

#if os(iOS)
import UIKit
#endif

/// A scope that asks the system for extra execution time while a body runs.
@MainActor
enum UploadLifeline {

    /// Run `body` with the app held awake, if the platform can do that.
    ///
    /// The identifier is always ended, on every path including expiry — an
    /// unended background task is a watchdog termination, which is a worse
    /// bug than the one this fixes. (`body` cannot throw: the send path it
    /// wraps reports failure by returning false.)
    static func withLifeline<T>(
        name: String = "FamilyConnect.upload",
        _ body: () async -> T
    ) async -> T {
        #if os(iOS)
        var identifier: UIBackgroundTaskIdentifier = .invalid
        identifier = UIApplication.shared.beginBackgroundTask(withName: name) {
            // Expiry handler: the allowance ran out with the upload still
            // running. Ending the identifier is the part the system
            // requires, and all that is done here — cancelling would race a
            // completion that may already have happened, and there is
            // nothing useful to cancel INTO, since a half-finished send has
            // nowhere to be saved.
            //
            // Logged because this is the one silent-loss path in the send
            // pipeline: the sender saw "Uploading 3 of 5…" and will get no
            // bubble and, if they have left the chat, no error either.
            AppLog.sync.error(
                "Upload background time expired; a send in flight is being abandoned")
            if identifier != .invalid {
                UIApplication.shared.endBackgroundTask(identifier)
                identifier = .invalid
            }
        }
        defer {
            if identifier != .invalid {
                UIApplication.shared.endBackgroundTask(identifier)
                identifier = .invalid
            }
        }
        return await body()
        #else
        return await body()
        #endif
    }
}
