//
//  ComposerRow.swift
//  FamilyConnect
//
//  The message row and the column it sits in — issue #106,
//  docs/composer-and-wide-layout-2026-10-20.md, sections 2.1 to 2.3. The
//  rules both Apple composers draw, as pure functions, so that a test can
//  pin them without a composer on screen and the phone, the iPad and the Mac
//  cannot drift apart again without one of those tests noticing.
//
//  THE ROW, left to right: the paperclip, alone; the field, with its glyphs
//  at its trailing edge INSIDE it — the stickers always (empty or typing,
//  so nothing moves under the thumb), then the video-message button while
//  the field is empty (#79, S1.4) or, in the assistant's chat, the
//  paintbrush; and the microphone/Send slot, which never moves (S1.3).
//  `@ai` is not in the row any more: it is the first line of the `@` list
//  and the paperclip's "Ask the Assistant" (protocol.md, "Mentioning a
//  member", amended 2026-10-20).
//
//  THE COLUMN: messages AND composer share one centred column, capped at
//  1024, and a bubble inside it at 560 — #44's 560-pt cap moved from the
//  container to the bubble, which is where its concern (a phone-sized card
//  inside a pane-wide balloon) actually lived.
//
//  THE KEYBOARD: a HARDWARE Return sends and Shift+Return breaks the line,
//  as on the Mac, the web and Windows; an on-screen keyboard's Return is
//  still a new line, because it never submits a multi-line field — it
//  arrives as typed text (HardwareReturn has the measurements).
//

import CoreGraphics

nonisolated enum ComposerRow {

    // MARK: - What sits inside the field

    /// A glyph inside the field, at its trailing edge, in drawing order
    /// (leading to trailing).
    enum FieldGlyph: Equatable, Hashable, Sendable {
        /// The family's stickers — wherever a message can be written and the
        /// server and the chat have them (`StickerDoor`), empty or typing.
        case stickers
        /// The assistant chat's `/draw` button. That chat has no microphone
        /// and no video button, so it has the room; and the 2026-10-07
        /// decision that it is a button and not a menu line stands.
        case paintbrush
        /// The video-message button (S1.4): only while the slot is a
        /// microphone, which is to say only while the field is empty.
        case video
    }

    /// The glyphs inside the field, in order. The stickers come first so
    /// that the glyph that comes and goes — the video button, at the first
    /// character typed — is the OUTERMOST one, and the stickers never shift
    /// sideways when it leaves.
    static func fieldGlyphs(stickers: Bool, paintbrush: Bool, video: Bool) -> [FieldGlyph] {
        var glyphs: [FieldGlyph] = []
        if stickers { glyphs.append(.stickers) }
        // The paintbrush lives in the assistant's chat, the video button
        // everywhere else; never both, and if a future server offered both,
        // the paintbrush is the one that is always there, so it goes inside.
        if paintbrush { glyphs.append(.paintbrush) }
        if video { glyphs.append(.video) }
        return glyphs
    }

    /// The room the field's text run gives up to its glyphs, measured from
    /// the capsule's trailing edge: the glyphs, the gaps between them and
    /// the inset after the last — or the plain inset when there are none.
    ///
    /// The doc's iPhone arithmetic, with the phone's numbers (a 30-pt glyph,
    /// 2 between, 6 after the last, 12 when there is nothing): a 281-pt
    /// capsule leaves 233 for text while typing and 201 while empty.
    static func trailingInset(glyphs: Int, glyphWidth: CGFloat, spacing: CGFloat,
                              endInset: CGFloat, bareInset: CGFloat) -> CGFloat {
        guard glyphs > 0 else { return bareInset }
        return CGFloat(glyphs) * glyphWidth + CGFloat(glyphs - 1) * spacing + endInset
    }

    /// How far a control's hit area reaches past its box on each side to
    /// make a 44-pt target (S1.1 "Targets"): 4 around the iPhone's 36-pt
    /// paperclip and Send, 7 around a 30-pt glyph in the field, none once
    /// Dynamic Type has made the box that big itself. The reach goes into
    /// the gaps between controls, so the row pays nothing for it in width.
    static func targetOverhang(side: CGFloat) -> CGFloat {
        max(0, (CGFloat(RecordRules.minTargetApplePT) - side) / 2)
    }

    // MARK: - Large text: the field on a line of its own

    /// The shortest text run the field may be left with before it takes a
    /// line of its own above the controls (2.1, "Dynamic Type"), at the
    /// DEFAULT text size. Below it a field is a slot to type blind into.
    ///
    /// The views scale it with the text (`@ScaledMetric`), and that is a
    /// reading of the doc's rule rather than a departure from it: 120 points
    /// is about seven characters of body text, and seven characters is what
    /// the rule protects. Held fixed, it kept the iPhone 17's row on one
    /// line at AX3 with a 121-pt field showing "Mess…" in 40-pt type —
    /// measured, ComposerLayoutUITests — exactly the field the rule exists
    /// to prevent. Scaled, the field takes its own line from AX2 up on a
    /// 393-pt phone and stays inline through AX1.
    static let shortestTextRun: CGFloat = 120

    /// How the row is arranged at a width.
    enum Arrangement: Equatable, Sendable {
        /// One line: paperclip, field, slot.
        case inline
        /// The field across the full width on top; the paperclip and the
        /// slot below it, at the two ends.
        case stacked
    }

    /// Inline while the paperclip, the slot, the gaps and the field's own
    /// chrome still leave `shortestTextRun` for text; stacked otherwise.
    ///
    /// - Parameters:
    ///   - width: the row's width, insets already taken off.
    ///   - paperclip / slot: their frames' widths (they scale with Dynamic
    ///     Type, which is exactly why the answer changes).
    ///   - fieldChrome: what the field spends on anything that is not text —
    ///     its leading padding plus `trailingInset`.
    ///   - textRun: the shortest text run allowed — `shortestTextRun`,
    ///     scaled with the text size by the caller.
    static func arrangement(width: CGFloat, paperclip: CGFloat, slot: CGFloat,
                            spacing: CGFloat, fieldChrome: CGFloat,
                            textRun: CGFloat = shortestTextRun) -> Arrangement {
        let needed = paperclip + spacing + fieldChrome + textRun + spacing + slot
        return width + 0.5 >= needed ? .inline : .stacked
    }

    // MARK: - The column (2.2)

    /// The widest the conversation column gets: messages and composer alike.
    static let maxColumn: CGFloat = 1024
    /// The widest a bubble gets, at any width — #44's 560, on the bubble.
    static let maxBubble: CGFloat = 560
    /// The room a bubble always leaves on its far side — the phone's spacer
    /// (`MessageBubbleView`), so the iPhone's bubble is exactly what it was.
    static let bubbleGutter: CGFloat = 48

    /// The least margin between the column and the pane's edges, per
    /// platform. The rule is "pane − 2 × margin, capped", and the margin is
    /// the inset each platform's thread ALREADY had: 12 on the iPhone and
    /// the iPad, so that every iPhone keeps the column it shipped with to
    /// the point (its bubbles are limited by the 48-pt spacer, and a wider
    /// margin would have narrowed them by 8); 16 on the Mac, the doc's own.
    /// The iPad 13" in landscape with its sidebar shown (a 1056-pt detail)
    /// comes out at exactly 1024 either way.
    static let phoneMargin: CGFloat = 12
    static let desktopMargin: CGFloat = 16

    /// This platform's margin.
    static var margin: CGFloat {
        #if os(macOS)
        desktopMargin
        #else
        phoneMargin
        #endif
    }

    /// The cap a centred container is given so that, with `margin` padded
    /// inside it, what is left is `column(pane:)` — the one number the
    /// views need, since a frame cap and an inner padding compose into
    /// exactly the rule with no measured width at all (no state, no second
    /// layout pass, nothing for #43/#44's anchoring to trip over).
    static func containerCap(margin: CGFloat = margin) -> CGFloat {
        maxColumn + 2 * margin
    }

    /// The conversation column in a pane this wide: `min(pane − 2 × margin,
    /// 1024)`, never negative.
    static func column(pane: CGFloat, margin: CGFloat = margin) -> CGFloat {
        max(0, min(pane - 2 * margin, maxColumn))
    }

    /// A bubble's widest in a column this wide: `min(column − 48, 560)`.
    static func bubble(column: CGFloat) -> CGFloat {
        max(0, min(column - bubbleGutter, maxBubble))
    }

    // MARK: - Return (2.3)

    /// What a Return from a HARDWARE keyboard does in the main composer.
    enum ReturnAction: Equatable, Sendable {
        /// Activate the slot — Send, Save, send the voice message or stop it.
        case activateSlot
        /// Leave it to the field, which breaks the line.
        case newLine
        /// Swallow it: nothing to send, and an empty field must not grow a
        /// blank line that the slot would then read as "not empty".
        case nothing
        /// ⌘↩ — leave it to the hidden ⌘↩ button, which has always been the
        /// iPad's Send and still is.
        case shortcut
    }

    /// Return alone activates the slot on rows 2 to 5 (S1.3) and nothing
    /// else — never a microphone: Return in an empty field never records.
    /// Shift+Return is a new line, and so are ⌥ and ⌃ with it, the Mac's
    /// answer (NewLineKeys) for whoever's fingers learned it there. ⌘↩ is
    /// the shortcut's.
    ///
    /// - Parameter slotTakesReturn: `ComposerSlot.takesReturn`, less a Save
    ///   that is disabled (the field is blank during an edit).
    static func hardwareReturn(shift: Bool, option: Bool, command: Bool, control: Bool,
                               slotTakesReturn: Bool) -> ReturnAction {
        if command { return .shortcut }
        if NewLineKeys.returnMakesLine(shift: shift, option: option, command: false, control: control) {
            return .newLine
        }
        return slotTakesReturn ? .activateSlot : .nothing
    }

    /// The same question for the thread's composer, which has no slot: a
    /// Send that is enabled or not.
    static func threadHardwareReturn(shift: Bool, option: Bool, command: Bool, control: Bool,
                                     canSend: Bool) -> ReturnAction {
        hardwareReturn(shift: shift, option: option, command: command, control: control,
                       slotTakesReturn: canSend)
    }
}
