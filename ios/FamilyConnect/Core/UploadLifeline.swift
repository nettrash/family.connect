//
//  UploadLifeline.swift
//  FamilyConnect
//
//  Keeps the app alive long enough to finish an attachment upload that was
//  already started when the person left.
//
//  WHY THIS EXISTS. Somebody presses Send and immediately switches apps.
//  The seconds a departing app is given are usually enough for a photo,
//  and finishing an upload already in progress beats starting it again
//  later on a colder cache.
//
//  WHAT IT IS NOT, ANY MORE. This file used to be the only thing standing
//  between a backgrounded app and a lost send, because a media send lived
//  nowhere but a running Task. Both halves of that are gone: a send is a
//  row plus staged bytes before the first byte leaves (`sendMedia`,
//  `PendingMediaItemEntity`), and the uploads a departing app still owes
//  are handed to the system itself (`BackgroundUploads`), which keeps
//  going while the app is suspended or dead. So losing the allowance now
//  postpones nothing that matters: the system is carrying the bytes, and
//  the ordinary outbox finishes the message on whatever trigger comes
//  next (docs/protocol.md, "Sending on an unreliable network").
//
//  iOS only. macOS does not suspend an app for being in the background, so
//  there is nothing to hold open; the no-op keeps `sendMedia` free of
//  platform conditionals.
//
//  Android counterpart: MediaUploadWorker.kt — WorkManager plays the part
//  `BackgroundUploads` plays here, and a coroutine on the application
//  scope plays this one.
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
