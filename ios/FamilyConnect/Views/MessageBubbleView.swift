//
//  MessageBubbleView.swift
//  FamilyConnect
//
//  One bubble. Mine: trailing, tint background, white text. Theirs:
//  leading, secondarySystemFill — semantic colors so both appearances
//  work without a palette. Emoji-only messages render bare: transparent
//  balloon, glyphs on the EmojiOnly size ladder (identical on Android).
//  Text bodies go through MessageLinks: URLs, emails and phone numbers
//  render as tappable links (browser / Mail / call).
//  Below the bubble: HH:mm plus, on own
//  messages, the delivery glyph ladder —
//
//    clock            pending (optimistic row, not yet on the server)
//    checkmark        on the server (ack/echo/REST confirmed)
//    double checkmark someone else read it (serverID ≤ othersReadUpTo),
//                     tinted; composed from two offset SF checkmarks
//                     because the system font has no double-check glyph
//    red exclamation  failed — tap to retry; long-press for the sheet
//                     with Retry/Delete
//
//  The sender-name caption (family chat, sender change — rules in
//  MessagePresentation) renders above the bubble in the accent color.
//
//  Reactions: aggregated chips (emoji + count, tinted when the current
//  user is included — aggregation rules in MessagePresentation) render
//  between the bubble and the timestamp in a wrapping FlowLayout, so
//  many chips fold to new lines instead of overflowing. Tapping a chip
//  toggles that emoji; long-pressing a chip pops the "who reacted" list
//  (rows from MessagePresentation.reactionDetails, passed in by the
//  parent). Chips spring in and out keyed by their emoji, counts roll
//  with numericText, and a soft impact fires when the current user's own
//  reaction state changes — i.e. when their toggle lands.
//
//  Long-pressing the bubble itself calls `onLongPress` — the parent
//  floats its reaction picker over the bubble, finding it through the
//  BubbleAnchorKey bounds this view publishes under its localID.
//  Double-tapping an acked bubble toggles the quick heart
//  (MessagePresentation.doubleTapReaction), same as picking ❤️ from
//  the capsule.
//

import SwiftUI

struct MessageBubbleView: View {
    let message: MessageSnapshot
    let isMine: Bool
    let showsSenderName: Bool
    let senderName: String?
    let isRead: Bool
    var reactionChips: [ReactionChip] = []
    var reactionDetails: [ReactionDetail] = []
    var onRetry: () -> Void = {}
    var onDelete: () -> Void = {}
    var onToggleReaction: (String) -> Void = { _ in }
    var onLongPress: () -> Void = {}
    /// True only while THIS bubble hosts the floating reaction picker.
    /// Anchors are position-dependent, so publishing them from every
    /// bubble makes the overlay's preference re-evaluate on every
    /// scrolled frame — with tall bubbles that is real jank. Only the
    /// pressed bubble's anchor is ever read, so only it publishes.
    var publishesAnchor: Bool = false

    /// Drives the "who reacted" popover a chip long-press opens.
    @State private var showsReactionDetails = false

    /// The in-flight deferred link open (see handleLinkTap). Non-nil
    /// exactly while a first link-tap waits out the double-tap window.
    @State private var pendingLinkOpen: Task<Void, Never>?

    /// The environment's own openURL, captured BEFORE the bubble
    /// overrides it — the override defers into this, never into itself.
    @Environment(\.openURL) private var systemOpenURL

    /// Dynamic Type factor for the emoji ladder: base 1, scaled with the
    /// body style, so the wrapped value IS the current body-relative
    /// scale. A fixed .system(size:) would invert the feature at
    /// accessibility sizes (emoji-only bubbles smaller than scaled body
    /// text); Android gets the same scaling for free from sp units.
    @ScaledMetric(relativeTo: .body) private var emojiFontScale: CGFloat = 1

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if isMine { Spacer(minLength: 48) }

            VStack(alignment: isMine ? .trailing : .leading, spacing: 2) {
                if showsSenderName, let senderName {
                    Text(senderName)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.tint)
                        .padding(.horizontal, 4)
                }

                bubble

                if !reactionChips.isEmpty {
                    reactionRow
                }

                HStack(spacing: 4) {
                    Text(message.createdAt, format: .dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if isMine {
                        statusGlyph
                    }
                }
                .padding(.horizontal, 4)
            }

            if !isMine { Spacer(minLength: 48) }
        }
        // A toggle landing = the set of chips that include me changing —
        // whether from the optimistic write or the server's echo.
        .sensoryFeedback(.impact(flexibility: .soft), trigger: reactionChips.filter(\.includesMe).map(\.emoji))
    }

    /// The aggregated reaction chips under the bubble, wrapping to new
    /// lines when they outgrow the column. A chip the current user is
    /// part of gets the tinted treatment; tapping any chip toggles that
    /// emoji for the current user; long-pressing one pops who reacted.
    private var reactionRow: some View {
        FlowLayout(rowAlignment: isMine ? .trailing : .leading, spacing: 4) {
            ForEach(reactionChips) { chip in
                Button {
                    onToggleReaction(chip.emoji)
                } label: {
                    HStack(spacing: 3) {
                        Text(chip.emoji)
                            .font(.footnote)
                        if chip.count > 1 {
                            Text("\(chip.count)")
                                .font(.caption2.weight(.medium))
                                .foregroundStyle(chip.includesMe ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                                .contentTransition(.numericText())
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(
                        chip.includesMe ? AnyShapeStyle(.tint.opacity(0.15)) : AnyShapeStyle(Color(.secondarySystemFill)),
                        in: Capsule())
                    .overlay {
                        if chip.includesMe {
                            Capsule().strokeBorder(.tint, lineWidth: 1)
                        }
                    }
                }
                .buttonStyle(.plain)
                // Simultaneous so the Button's tap keeps working; being
                // deeper than the bubble's own long-press, this one wins
                // when the press starts on a chip.
                .simultaneousGesture(LongPressGesture().onEnded { _ in
                    showsReactionDetails = true
                })
                .transition(.scale.combined(with: .opacity))
                .accessibilityLabel("\(chip.emoji) \(chip.count)")
            }
        }
        .padding(.horizontal, 4)
        .padding(.top, 1)
        // Scoped to the chip row: an animation watching the whole bubble
        // row's frame kept a spring alive against layout re-passes.
        .animation(.spring(duration: 0.25, bounce: 0.3), value: reactionChips)
        .popover(isPresented: $showsReactionDetails) {
            reactionDetailsList
                .presentationCompactAdaptation(.popover)
        }
    }

    /// The "who reacted" popover body: each emoji in chip order with the
    /// names of its reactors ("You" first when the current user is one).
    private var reactionDetailsList: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(reactionDetails) { detail in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(detail.emoji)
                    Text(detail.names.formatted(.list(type: .and)))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(12)
    }

    private var bubble: some View {
        // Emoji-only messages render bare: no balloon, just the glyphs
        // (the padding stays so the long-press target and the picker
        // anchor keep their size). Foreground goes primary on both
        // sides — the rare monochrome pictograph (☂, ™) would vanish
        // as white-on-transparent on own messages. Text messages go
        // through MessageLinks so URLs, emails and phone numbers are
        // tappable; emoji-only ones skip the detector.
        let isEmojiOnly = EmojiOnly.displayFontSize(for: message.body) != nil
        return Text(
            isEmojiOnly
                ? AttributedString(message.body)
                : MessageLinks.attributedBody(message.body, isMine: isMine))
            .font(bubbleFont)
            .foregroundStyle(
                !isEmojiOnly && isMine ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                isEmojiOnly
                    ? AnyShapeStyle(Color.clear)
                    : isMine ? AnyShapeStyle(.tint) : AnyShapeStyle(Color(.secondarySystemFill)),
                in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            // The floating reaction picker grows out of this exact rect;
            // the parent resolves it from the preference by localID. Empty
            // unless this bubble is the picker's host (see publishesAnchor).
            .anchorPreference(key: BubbleAnchorKey.self, value: .bounds) {
                publishesAnchor ? [message.localID: $0] : [:]
            }
            .onTapGesture(count: 2) {
                // Double-tap = the quick heart (Tapback idiom), through
                // the same toggle path as the capsule, so a second
                // double-tap removes it. Only reachable over NON-link
                // glyphs — Text's internal link tap preempts this
                // gesture, so over links the heart comes from
                // handleLinkTap's own double-tap arbitration.
                toggleQuickHeart()
            }
            .onLongPressGesture {
                onLongPress()
            }
            // Text fires link taps through the environment's openURL —
            // and, measured on device: it fires them for EACH tap of a
            // double-tap while the count-2 gesture above never runs over
            // link glyphs. So the override below is the arbitration
            // layer: defer every open past the double-tap window, and
            // treat a second fire inside the window as the heart.
            .environment(\.openURL, OpenURLAction { url in
                handleLinkTap(url)
                return .handled
            })
    }

    /// One tap on a link run. First fire: schedule the open after the
    /// double-tap window. Second fire inside the window: the user is
    /// double-tapping link glyphs — cancel the open and heart instead
    /// (falling back to opening when the bubble cannot take a reaction
    /// yet, so a pending bubble's links never go dead).
    private func handleLinkTap(_ url: URL) {
        if pendingLinkOpen != nil {
            pendingLinkOpen?.cancel()
            pendingLinkOpen = nil
            if message.serverID != nil {
                toggleQuickHeart()
            } else {
                systemOpenURL(url)
            }
            return
        }
        pendingLinkOpen = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled else { return }
            pendingLinkOpen = nil
            systemOpenURL(url)
        }
    }

    /// The double-tap reaction, gated exactly like the capsule: a
    /// reaction needs a server message id.
    private func toggleQuickHeart() {
        guard message.serverID != nil else { return }
        onToggleReaction(MessagePresentation.doubleTapReaction)
    }

    /// Emoji-only messages render on the EmojiOnly size ladder (one
    /// emoji biggest through four smallest), scaled with Dynamic Type;
    /// everything else is body text. Same ladder as Android.
    private var bubbleFont: Font {
        if let size = EmojiOnly.displayFontSize(for: message.body) {
            return .system(size: size * emojiFontScale)
        }
        return .body
    }

    @ViewBuilder
    private var statusGlyph: some View {
        switch message.state {
        case .pending:
            Image(systemName: "clock")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .accessibilityLabel("Sending")
        case .sent:
            if isRead {
                DoubleCheckmark()
                    .foregroundStyle(.tint)
                    .accessibilityLabel("Read")
            } else {
                Image(systemName: "checkmark")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Sent")
            }
        case .failed:
            Button {
                onRetry()
            } label: {
                HStack(spacing: 2) {
                    Image(systemName: "exclamationmark.circle.fill")
                    Text("Tap to retry")
                }
                .font(.caption2)
                .foregroundStyle(.red)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Failed. Tap to retry.")
        }
    }
}

/// Two overlapped SF checkmarks — the system has no double-check symbol.
private struct DoubleCheckmark: View {
    var body: some View {
        ZStack(alignment: .leading) {
            Image(systemName: "checkmark")
            Image(systemName: "checkmark")
                .offset(x: 4)
        }
        .font(.caption2)
        .padding(.trailing, 4)
    }
}
