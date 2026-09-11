//
//  BackgroundUploads.swift
//  FamilyConnect
//
//  Finishing an attachment upload after somebody has left the app
//  (docs/protocol.md, "Sending on an unreliable network": an upload may be
//  handed to the system and land while the app is not running).
//
//  WHAT THIS ADDS TO WHAT WAS ALREADY THERE. A media send is already
//  durable: `sendMedia` writes the message row and its item rows and moves
//  the bytes into `PendingMediaStaging` before the first byte leaves, and
//  the ordinary outbox resumes it on the socket connecting, the foreground
//  edge, the network returning and its own wake timer. What none of that
//  covers is the case the issue is about: somebody presses Send on a 90 MB
//  video and switches apps. `UploadLifeline` buys seconds; iOS then
//  suspends the process and the bytes wait for the next foreground, which
//  may be tomorrow.
//
//  A BACKGROUND `URLSession` IS THE ONLY THING ON THIS PLATFORM THAT
//  KEEPS UPLOADING. The system owns the transfer: it survives suspension,
//  it survives the app being killed, it waits for a network of its own
//  accord, and it relaunches the app in the background to hand back the
//  answer (`sessionSendsLaunchEvents`, handled in `AppDelegate`).
//
//  WHAT IT DOES AND DOES NOT CARRY. The BYTES — the slow half — go to the
//  system. The preview PUT and the claiming `POST /chats/{id}/messages`
//  stay in the app's own leg, because they are small and because a message
//  must be posted only once every id is in hand. So the sequence for a
//  send somebody walked away from is: the video lands while they are
//  elsewhere, the id is written onto the item row from the delegate, and
//  the next run of the outbox — the app relaunched for the session's own
//  events, or opened by a person — posts the message with nothing left to
//  upload. That is the resumability the item rows were built for, used by
//  a second uploader.
//
//  ONE UPLOADER PER ITEM. The app's own leg skips an item this session
//  already holds a task for (`inFlightItemIDs`), because two uploads of
//  the same bytes would leave the server holding two unclaimed copies —
//  harmless (the sweep takes the loser) but wasteful of somebody's data.
//
//  iOS ONLY. macOS does not suspend an app for being in the background, so
//  the in-process leg is already the right answer there and this file
//  compiles to nothing.
//
//  Android counterpart: MediaUploadWorker.kt, which hands the same work to
//  WorkManager.
//

import Foundation
import os

#if os(iOS)

/// The system's own uploader, for attachments whose send has to outlive the
/// app.
@MainActor
final class BackgroundUploads: NSObject {

    /// One item's bytes, ready to hand over.
    struct Handover: Sendable {
        let itemID: String
        let request: URLRequest
        let fileURL: URL
    }

    static let shared = BackgroundUploads()

    /// Stable across launches, because that is what lets a relaunched app
    /// reclaim the transfers the last one started.
    static let identifier = "me.nettrash.familyconnect.uploads"

    private let log = Logger(subsystem: AppLog.subsystem, category: "upload.bg")

    /// Where a landed upload is written down. Set once, by whoever owns the
    /// store; a `nil` here means an answer arrived before the app was ready
    /// for it, and the transfer is simply left for the in-process leg.
    var landed: ((_ itemID: String, _ attachment: AttachmentDTO) -> Void)?

    /// Called when the system has finished handing back everything it did
    /// while the app was away — the moment to run the outbox, because the
    /// ids that were missing may all be there now.
    var finishedEvents: (() -> Void)?

    /// The completion handler iOS gives the app when it relaunches it for
    /// this session. Calling it is not optional: an app that does not is
    /// eventually stopped from using background sessions at all.
    private var systemCompletion: (() -> Void)?

    /// Response bytes per task, because a body arrives in pieces and the
    /// id is in it.
    private var bodies: [Int: Data] = [:]

    /// Built on first use, which is also what adopts the transfers a
    /// previous process started — and rebuilt after a `cancelAll`, because
    /// an invalidated session is dead for good and the next account signing
    /// in on this device still has sends to finish.
    private var held: URLSession?

    private var session: URLSession {
        if let held { return held }
        let configuration = URLSessionConfiguration.background(withIdentifier: Self.identifier)
        // Not discretionary: somebody pressed Send. The system may still
        // wait for a better network, but it will not hold the transfer for
        // a charger.
        configuration.isDiscretionary = false
        configuration.sessionSendsLaunchEvents = true
        configuration.allowsCellularAccess = true
        let session = URLSession(
            configuration: configuration, delegate: self, delegateQueue: .main)
        held = session
        return session
    }

    /// Adopt whatever is still running from a previous process. Cheap, and
    /// it must happen before the first `hand(over:)` so the dedup below can
    /// see those tasks.
    func adopt() async {
        _ = await session.allTasks
    }

    /// The items this session is already uploading. The in-process leg asks
    /// so it never uploads the same bytes twice.
    func inFlightItemIDs() async -> Set<String> {
        Set(await session.allTasks.compactMap { $0.taskDescription })
    }

    /// Give the system the bytes it should finish whatever happens to the
    /// app. Items it is already carrying are left alone.
    func hand(over items: [Handover]) async {
        guard !items.isEmpty else { return }
        let already = await inFlightItemIDs()
        for item in items where !already.contains(item.itemID) {
            let task = session.uploadTask(with: item.request, fromFile: item.fileURL)
            // The only thing that survives a relaunch with the task, so it
            // is what says which row this upload belongs to.
            task.taskDescription = item.itemID
            task.resume()
            log.info("handed an upload to the system")
        }
    }

    /// Stop carrying anything. A send composed in one account must never
    /// reach the next, and a transfer the system is holding would outlive
    /// the sign-out that took its rows away.
    func cancelAll() {
        held?.invalidateAndCancel()
        held = nil
        bodies.removeAll()
    }

    /// Keep the system's completion handler until its events are done.
    func holdSystemCompletion(_ completion: @escaping () -> Void) {
        systemCompletion = completion
        // Touching the session is what starts the hand-back.
        _ = session
    }
}

// MARK: - What the system hands back

extension BackgroundUploads: URLSessionDataDelegate {

    nonisolated func urlSession(
        _ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data
    ) {
        MainActor.assumeIsolated {
            bodies[dataTask.taskIdentifier, default: Data()].append(data)
        }
    }

    nonisolated func urlSession(
        _ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?
    ) {
        MainActor.assumeIsolated {
            let body = bodies.removeValue(forKey: task.taskIdentifier)
            guard let itemID = task.taskDescription else { return }
            let status = (task.response as? HTTPURLResponse)?.statusCode ?? 0
            guard error == nil, (200..<300).contains(status), let body else {
                // Nothing is written down and nothing is failed: the item
                // still owes an upload, and the app's own leg — which can
                // tell a refusal from a dropped network and has the retry
                // ladder — decides what that means on the next trigger.
                log.warning("a background upload did not land (status \(status))")
                return
            }
            guard let decoded = try? APICoding.decoder().decode(
                AttachmentResponse.self, from: body)
            else {
                log.error("a landed upload answered something unreadable")
                return
            }
            landed?(itemID, decoded.attachment)
        }
    }

    nonisolated func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        MainActor.assumeIsolated {
            finishedEvents?()
            let completion = systemCompletion
            systemCompletion = nil
            completion?()
        }
    }
}

#endif
