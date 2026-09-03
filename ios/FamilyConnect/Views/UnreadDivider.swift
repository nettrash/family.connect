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
                // The 12pt of breathing room around the disc is INSIDE the
                // button and is part of what can be pressed, rather than
                // outside it as dead margin. Nothing moves: the padding was
                // always there, the disc is drawn in the same place, and the
                // control still occupies 60x60 of layout. What changes is
                // that the 60pt target is now the TAP target too.
                //
                // Two reasons, and the second is a bug.
                //
                // A 36pt control is under Apple's own 44pt minimum, which is
                // reason enough on a phone. But in a NavigationSplitView
                // detail column (iPad, issue #43) a 36pt control does not
                // receive a tap at its CENTRE at all — the one point a thumb
                // and XCUITest both aim at. Measured on a 13-inch iPad,
                // three controls in the same overlay in the same build, each
                // tapped at its own centre: this button (36pt) 0 hits out of
                // 2, the same disc driven by `.onTapGesture` instead of a
                // Button (36pt) 0 out of 2, and an otherwise identical
                // Button at 90pt 2 out of 2. Sweeping across the 36pt one in
                // 6pt steps found live points only at ±6 from the centre.
                // Under a NavigationStack — the phone, and the same iPad
                // before the split view — the centre landed every time (7 of
                // 7 across the button's width on an iPhone 17).
                //
                // So it is not the host (overlay or ZStack sibling), not the
                // gesture (Button or tap gesture), not the transition, not
                // the scroll indicator and not the button's position: all
                // five were tried and changed nothing. It is the size. This
                // button is the only way back to the newest message after
                // reading history, and ChatPresence only marks a chat read
                // once the newest message is on screen — so a reader who
                // cannot press it also keeps an unread badge they cannot
                // clear.
                .padding(12)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.tint)
        .shadow(color: .black.opacity(0.12), radius: 4, y: 2)
        .accessibilityLabel("Scroll to newest")
    }
}
