//
//  UnreadDivider.swift
//  FamilyConnect
//
//  The "N new messages" rule between the last message the reader has seen
//  and the first they have not, plus the way back down to the newest one.
//
//  Shared by both Apple threads with no `#if os` in it, the way DayPill
//  deliberately is not: the two day pills drifted into two spellings of
//  one thing, and this is the row directly under one of them. The
//  platform vocabulary it needs — a recessed fill, a hairline — is in
//  PlatformStyle.
//
//  Placement is decided by MessagePresentation.daySections (which row of
//  which section it sits above), from an anchor UnreadAnchor decided ONCE
//  at open. Neither is recomputed while the chat is on screen: the unread
//  count is zeroed by the read path the instant the reader reaches the
//  bottom, so anything derived every pass would erase itself exactly when
//  the reader arrived at it.
//

import SwiftUI

/// The rule with its count, drawn above the oldest unread message.
///
/// It carries a scroll `id` because it is the SCROLL TARGET of the
/// anchored open, not just decoration: aiming at the message below it and
/// asking for `.top` parks the divider one row off the top edge, so the
/// reader arrives at their unread messages with nothing to say where the
/// boundary was.
struct UnreadDivider: View {
    let count: Int

    /// The anchored open's scroll target. A constant, like the threads'
    /// bottom sentinels — there is at most one of these in a thread.
    static let scrollID = "unread-divider"

    var body: some View {
        HStack(spacing: 8) {
            line
            // The count is the chat's unread count as the store held it at
            // open, which is the same number the chat list's badge shows.
            // It can be a message or two out of step with the rows below
            // it — the count and the cache are two different instants (one
            // arriving live between the chat list being read and the page
            // being drawn moves one and not the other) — and agreeing with
            // the badge the reader just tapped is worth more than agreeing
            // with a row count nobody can see.
            Text("\(count) new messages")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.tint)
                .fixedSize()
            line
        }
        .padding(.vertical, 6)
        .accessibilityElement(children: .combine)
    }

    private var line: some View {
        Rectangle()
            .fill(Color.appSeparator)
            .frame(height: 1)
    }
}

/// The way back to the newest message, for a reader the anchored open has
/// deliberately left in history.
///
/// Android has had this button since before the anchored open existed; it
/// is ported rather than reinvented so the three clients agree about what
/// the gesture looks like and what it is called. Its trigger rule lives in
/// ThreadFollow with the rest of the thread's arithmetic.
struct JumpToNewestButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "chevron.down")
                .font(.system(size: 15, weight: .semibold))
                .frame(width: 36, height: 36)
                .background(.regularMaterial, in: Circle())
                .overlay(Circle().stroke(Color.appSeparator, lineWidth: 0.5))
        }
        .buttonStyle(.plain)
        .foregroundStyle(.tint)
        .shadow(color: .black.opacity(0.12), radius: 4, y: 2)
        .padding(12)
        .accessibilityLabel("Scroll to newest")
    }
}
