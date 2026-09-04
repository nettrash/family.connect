//
//  ChatPresence.swift
//  FamilyConnect
//
//  THE definition of "the user is looking at this chat right now", and
//  deliberately the only one in the app.
//
//  Two features ask that question and they must never answer it
//  differently. The unread rule asks it to decide whether an arriving
//  message is read or unread; the Mac's local notification asks it to
//  decide whether to raise a banner. Two independent answers would drift,
//  and the drift that costs something is the one where a message is
//  quietly marked read AND quietly not announced — a message nobody is
//  ever told about.
//
//  READ MEANS SEEN, which is three facts at the same moment and not one:
//  the conversation is open, its newest message is inside the viewport,
//  and the app (this window, on the Mac) is genuinely frontmost. Opening
//  a chat is not reading it. Being selected in a sidebar is not reading
//  it. The app coming back to the foreground is not reading it. A resync
//  is not reading it. None of those puts a word in front of anybody.
//
//  Why the bar is that high: the server's read marker is monotonic
//  (`GREATEST(last_read_message_id, EXCLUDED…)`, migration 0001), so a
//  read this app reports by mistake is permanent AND cross-device — no
//  resync, reinstall or cold start brings that badge back on any device
//  the person owns. Every judgement call here is therefore biased towards
//  NOT reading: a badge that lingers a moment too long costs a glance, a
//  badge cleared too early costs the message.
//
//  The same principle already governs the board badge, for the same
//  reason — see BoardBadge, whose marks refuse to ride the sync cursor
//  because a background resync would clear them for somebody who never
//  opened the board.
//

import Foundation

nonisolated struct ChatPresence: Equatable, Sendable {
    /// The conversation whose view holds the claim. One value for the whole
    /// app: on the Mac several conversation windows can be open at once and
    /// the frontmost one owns it (MacConversationView publishes on
    /// `controlActiveState`).
    let chatID: Int64

    /// The newest message is inside the viewport — from the bottom
    /// sentinel's real scroll geometry on both platforms, never from a
    /// guess about the scroll offset and never from a row merely having
    /// been created. A reader parked in last week's history has the chat
    /// open and sees nothing that arrives.
    let isAtNewest: Bool

    /// iOS: `scenePhase == .active`. macOS: `controlActiveState == .key`.
    /// A Mac holds its socket open behind every other window, so this is
    /// the difference between a message arriving in front of somebody and
    /// one arriving in a window they cannot see.
    let isFrontmost: Bool

    /// The whole question, asked of one chat.
    func isReading(_ chatID: Int64) -> Bool {
        self.chatID == chatID && isAtNewest && isFrontmost
    }
}

/// The opening act of a conversation view: claim the chat, fetch a page if
/// there is nothing to show, wait for the first layout pass, and only then
/// say what the reader can actually see.
///
/// It is a function rather than a `.task` closure body for ONE reason, and
/// the reason is cancellation. `.task(id:)` is cancelled when the view goes
/// away — a conversation window closed, or a sidebar selection changed,
/// which swaps the view's `.id` and tears the old one down — and cancelling
/// a task that is SLEEPING makes `Task.sleep` throw immediately. Written
/// `try? await Task.sleep(…)` that error is swallowed and the next line
/// runs anyway, in a view that no longer exists: `onDisappear` has already
/// released the claim, so the re-published one is accepted (nobody holds
/// it), it names the chat the window used to show, and `isAtNewest` is
/// still the optimistic `true` the view started with. Nothing ever releases
/// it again. From then on every message arriving in that chat is marked
/// read without being seen — permanently and on every device the person
/// owns, because the server's marker is monotonic — and on the Mac its
/// notification is suppressed too, since `announce` asks the same question.
///
/// So the wait is written to STOP when it is cancelled, and it lives here
/// where the cancellation rule can be tested: a torn-down window is not
/// something a unit test can arrange from a SwiftUI view.
@MainActor
enum ChatPresenceOpening {

    /// Long enough for the first layout pass to place the bottom sentinel
    /// and report real geometry, short enough that a reader who opened the
    /// chat at its newest message is not left with a stale claim.
    ///
    /// `nonisolated` so it can be the default argument below, which is
    /// evaluated at the call site rather than on this actor.
    nonisolated static let settleDelay: UInt64 = 300_000_000

    /// - Parameters:
    ///   - claim: take the presence claim, claiming NOTHING about what is
    ///     visible yet.
    ///   - loadOlder: fetch the first page, if the cache has nothing.
    ///   - place: put the thread where this open is meant to land — at the
    ///     oldest unread message, under its divider — and do nothing at all
    ///     for the ordinary open, which the scroll view's own bottom anchor
    ///     already handles. It runs AFTER `loadOlder` because the anchor is
    ///     arithmetic over cached rows, and BEFORE the settle wait because
    ///     what `settled` publishes has to be where the thread ended up,
    ///     not where it started. Its own sleeps are the caller's; this
    ///     function only guarantees a cancellation check on each side.
    ///   - settled: publish what the bottom sentinel now reports. Runs only
    ///     if the view is still there to speak for.
    static func run(
        settleDelay: UInt64 = Self.settleDelay,
        claim: () -> Void,
        loadOlder: () async -> Void,
        place: () async -> Void = {},
        settled: () -> Void
    ) async {
        claim()
        await loadOlder()
        // The page above is a network round trip — ample time for the
        // window to be closed while it is in flight.
        guard !Task.isCancelled else { return }
        await place()
        guard !Task.isCancelled else { return }
        // Not `try?`: the only thing this can throw is the cancellation
        // this whole function exists to respect.
        do {
            try await Task.sleep(nanoseconds: settleDelay)
        } catch {
            return
        }
        // ...and once more, for a close that lands between the sleep
        // finishing and this line getting the actor back.
        guard !Task.isCancelled else { return }
        settled()
    }
}
