//
//  ComposerRowTests.swift
//  FamilyConnectTests
//
//  The message row and the wide screens (#106,
//  docs/composer-and-wide-layout-2026-10-20.md): what sits inside the
//  field, the arithmetic the doc measured, when the field takes a line of
//  its own, the column and the bubble, a hardware Return, and the `@` list
//  with the assistant first (protocol.md, "Mentioning a member", amended
//  2026-10-20).
//

import CoreGraphics
import Testing
@testable import FamilyConnect

struct ComposerRowTests {

    // MARK: - The glyphs inside the field

    @Test("the family chat, empty: the stickers, then the video button outermost")
    func emptyFamilyChat() {
        #expect(ComposerRow.fieldGlyphs(stickers: true, paintbrush: false, video: true)
            == [.stickers, .video])
    }

    @Test("typing: the video button leaves and the stickers STAY (the row no longer moves)")
    func typingKeepsTheStickers() {
        #expect(ComposerRow.fieldGlyphs(stickers: true, paintbrush: false, video: false) == [.stickers])
    }

    @Test("the assistant chat: the paintbrush beside the stickers")
    func assistantChat() {
        #expect(ComposerRow.fieldGlyphs(stickers: true, paintbrush: true, video: false)
            == [.stickers, .paintbrush])
        // A server with no stickers still has its paintbrush.
        #expect(ComposerRow.fieldGlyphs(stickers: false, paintbrush: true, video: false) == [.paintbrush])
    }

    @Test("nothing to show: no glyphs")
    func bareField() {
        #expect(ComposerRow.fieldGlyphs(stickers: false, paintbrush: false, video: false).isEmpty)
    }

    @Test("the stickers keep their index whether the video button shows or not")
    func stickersNeverShift() {
        for paintbrush in [false, true] {
            let empty = ComposerRow.fieldGlyphs(stickers: true, paintbrush: paintbrush, video: true)
            let typing = ComposerRow.fieldGlyphs(stickers: true, paintbrush: paintbrush, video: false)
            #expect(empty.firstIndex(of: .stickers) == 0)
            #expect(typing.firstIndex(of: .stickers) == 0)
        }
    }

    // MARK: - The doc's iPhone arithmetic (2.1)

    /// iPhone 393: 393 − 24 bar insets − paperclip 36 − 8 − 8 − slot 36.
    private static let phoneCapsule: CGFloat = 393 - 24 - 36 - 8 - 8 - 36

    private static func textRun(glyphs: Int) -> CGFloat {
        phoneCapsule - 12 - ComposerRow.trailingInset(
            glyphs: glyphs, glyphWidth: 30, spacing: 2, endInset: 6, bareInset: 12)
    }

    @Test("iPhone 393, family chat: a 201-pt text run while empty, 233 while typing")
    func phoneTextRuns() {
        #expect(Self.phoneCapsule == 281)
        #expect(Self.textRun(glyphs: 2) == 201)
        #expect(Self.textRun(glyphs: 1) == 233)
        // With nothing inside, the field keeps its old symmetric inset:
        // 12 each side of a 281-pt capsule.
        let bare: CGFloat = 257
        #expect(Self.textRun(glyphs: 0) == bare)
    }

    // MARK: - Large text (2.1, "Dynamic Type")

    @Test("at the default text size the iPhone row is one line")
    func defaultSizeIsInline() {
        // 369 wide; the field spends 12 + 30 + 2 + 30 + 6 = 80 on chrome.
        #expect(ComposerRow.arrangement(
            width: 369, paperclip: 36, slot: 36, spacing: 8, fieldChrome: 80) == .inline)
    }

    @Test("at AX3 the field takes its own line — with the text run scaled as the text is")
    func largeTextStacks() {
        // The iPhone 17 at AX3, measured (ComposerLayoutUITests): the row's
        // @ScaledMetric boxes come out at ~2.1× — 36 → 75.6, 30 → 63.
        let k: CGFloat = 2.1
        let chrome = 12 + (30 * k + 6)
        #expect(ComposerRow.arrangement(
            width: 369, paperclip: 36 * k, slot: 36 * k, spacing: 8,
            fieldChrome: chrome, textRun: ComposerRow.shortestTextRun * k) == .stacked)
        // Why the run is scaled: held at a fixed 120 the row stayed inline
        // there, with a 121-pt field showing "Mess…" in 40-pt type.
        #expect(ComposerRow.arrangement(
            width: 369, paperclip: 36 * k, slot: 36 * k, spacing: 8,
            fieldChrome: chrome, textRun: ComposerRow.shortestTextRun) == .inline)
    }

    @Test("through xxxLarge (~1.35×) the phone's row stays on one line")
    func largestStandardSizeIsInline() {
        let k: CGFloat = 1.35
        #expect(ComposerRow.arrangement(
            width: 369, paperclip: 36 * k, slot: 36 * k, spacing: 8,
            fieldChrome: 12 + (30 * k + 6), textRun: ComposerRow.shortestTextRun * k) == .inline)
    }

    @Test("the switch is at exactly a 120-pt text run")
    func theThreshold() {
        let chrome: CGFloat = 80
        let exact = 36 + 8 + chrome + ComposerRow.shortestTextRun + 8 + 36
        #expect(ComposerRow.arrangement(width: exact, paperclip: 36, slot: 36, spacing: 8,
                                        fieldChrome: chrome) == .inline)
        #expect(ComposerRow.arrangement(width: exact - 1, paperclip: 36, slot: 36, spacing: 8,
                                        fieldChrome: chrome) == .stacked)
    }

    // MARK: - The column and the bubble (2.2)

    @Test("the iPad 13\" in landscape with its sidebar shown: a 1024 column and 560 bubbles")
    func iPadLandscapeWithSidebar() {
        let column = ComposerRow.column(pane: 1056, margin: ComposerRow.phoneMargin)
        #expect(column == 1024)
        #expect(ComposerRow.bubble(column: column) == 560)
    }

    @Test("wider panes are capped at 1024 and centred; narrower ones keep the margin")
    func columnRule() {
        #expect(ComposerRow.column(pane: 1376, margin: ComposerRow.phoneMargin) == 1024)
        #expect(ComposerRow.column(pane: 2560, margin: ComposerRow.desktopMargin) == 1024)
        // The Mac at 1280×800 with its sidebar: a 1000-pt detail.
        #expect(ComposerRow.column(pane: 1000, margin: ComposerRow.desktopMargin) == 968)
        // Never negative, whatever a split view hands over.
        #expect(ComposerRow.column(pane: 10, margin: ComposerRow.desktopMargin) == 0)
    }

    @Test("every iPhone keeps the column and the bubble it had: the pane less 24, the bubble less 48")
    func iPhonesUnchanged() {
        for pane: CGFloat in [375, 390, 393, 402, 430, 440] {
            let column = ComposerRow.column(pane: pane, margin: ComposerRow.phoneMargin)
            #expect(column == pane - 24)
            #expect(ComposerRow.bubble(column: column) == pane - 24 - 48)
        }
    }

    @Test("the bubble's cap binds from a 608-pt column up")
    func bubbleCap() {
        #expect(ComposerRow.bubble(column: 608) == 560)
        #expect(ComposerRow.bubble(column: 607) == 559)
        #expect(ComposerRow.bubble(column: 2000) == 560)
        #expect(ComposerRow.bubble(column: 20) == 0)
    }

    @Test("the container cap composes with its inner margin into exactly the column rule")
    func containerCapComposes() {
        for margin in [ComposerRow.phoneMargin, ComposerRow.desktopMargin] {
            let cap = ComposerRow.containerCap(margin: margin)
            for pane: CGFloat in [320, 393, 744, 874, 1000, 1056, 1366, 1920] {
                // What a `.frame(maxWidth: cap)` with `margin` padded inside
                // it leaves for content in a pane this wide.
                let content = min(pane, cap) - 2 * margin
                #expect(content == ComposerRow.column(pane: pane, margin: margin))
            }
        }
    }

    // MARK: - A hardware Return (2.3)

    private static func press(
        shift: Bool = false, option: Bool = false, command: Bool = false, control: Bool = false,
        takes: Bool = true
    ) -> ComposerRow.ReturnAction {
        ComposerRow.hardwareReturn(
            shift: shift, option: option, command: command, control: control, slotTakesReturn: takes)
    }

    @Test("Return alone sends where the slot takes Return (rows 2 to 5)")
    func returnSends() {
        #expect(Self.press() == .activateSlot)
    }

    @Test("Return in an empty field never records, and leaves no blank line behind (S1.3)")
    func returnNeverRecords() {
        #expect(Self.press(takes: false) == .nothing)
    }

    @Test("Shift+Return is a new line — and so are ⌥ and ⌃, the Mac's habit (NewLineKeys)")
    func modifiedReturnBreaksTheLine() {
        #expect(Self.press(shift: true) == .newLine)
        #expect(Self.press(option: true) == .newLine)
        #expect(Self.press(control: true) == .newLine)
        // Even where there is nothing to send: a line in a draft is a line.
        #expect(Self.press(shift: true, takes: false) == .newLine)
    }

    @Test("⌘↩ is left to the shortcut that has always been the iPad's Send")
    func commandReturnIsTheShortcuts() {
        #expect(Self.press(command: true) == .shortcut)
        #expect(Self.press(shift: true, command: true) == .shortcut)
        #expect(Self.press(command: true, takes: false) == .shortcut)
    }

    @Test("the slot's own answer feeds it: rows 2–5 take Return, a disabled Save and a microphone do not")
    func slotRowsFeedThePolicy() {
        func takes(_ slot: ComposerSlot) -> Bool { slot.takesReturn && slot != .save(enabled: false) }
        #expect(takes(.send))
        #expect(takes(.sendVoice))
        #expect(takes(.stopRecording))
        #expect(takes(.save(enabled: true)))
        #expect(!takes(.save(enabled: false)))
        #expect(!takes(.microphone))
        #expect(!takes(.dimmed(.call)))
        #expect(!takes(.sendDisabled))
    }

    @Test("the thread's Return: Send while there is something to send, nothing while blank")
    func threadReturn() {
        #expect(ComposerRow.threadHardwareReturn(
            shift: false, option: false, command: false, control: false, canSend: true) == .activateSlot)
        #expect(ComposerRow.threadHardwareReturn(
            shift: false, option: false, command: false, control: false, canSend: false) == .nothing)
        #expect(ComposerRow.threadHardwareReturn(
            shift: true, option: false, command: false, control: false, canSend: true) == .newLine)
    }

    // MARK: - Targets (S1.1)

    @Test("44-pt targets: 4 around a 36-pt control, 7 around a 30-pt glyph, none once it is 44")
    func targets() {
        #expect(ComposerRow.targetOverhang(side: 36) == 4)
        #expect(ComposerRow.targetOverhang(side: 30) == 7)
        #expect(ComposerRow.targetOverhang(side: 44) == 0)
        #expect(ComposerRow.targetOverhang(side: 64) == 0)
    }

    // MARK: - The `@` list, the assistant first

    private static let roster = [
        MentionDTO(userID: 1, name: "Nora"),
        MentionDTO(userID: 2, name: "Dan"),
        MentionDTO(userID: 3, name: "Ellie"),
        MentionDTO(userID: 4, name: "Aiden"),
    ]

    private static func offered(_ query: String, assistant: String? = "Assistant") -> [MentionSuggestion] {
        MemberMentions.suggestions(in: roster, matching: query, excluding: [1], assistantName: assistant)
    }

    @Test("a bare `@` lists the assistant FIRST, then the members in roster order")
    func bareAtListsTheAssistantFirst() {
        #expect(Self.offered("") == [
            .assistant(name: "Assistant"),
            .member(MentionDTO(userID: 2, name: "Dan")),
            .member(MentionDTO(userID: 3, name: "Ellie")),
            .member(MentionDTO(userID: 4, name: "Aiden")),
        ])
    }

    @Test("narrowed by its display name, as a member is by theirs")
    func narrowedByName() {
        #expect(Self.offered("as") == [.assistant(name: "Assistant")])
        #expect(Self.offered("ASSIST").first == .assistant(name: "Assistant"))
        #expect(Self.offered("d") == [.member(MentionDTO(userID: 2, name: "Dan"))])
    }

    @Test("narrowed by `ai` too, whatever it is called — and a member starting `Ai` still shows")
    func narrowedByTheToken() {
        let named = Self.offered("ai", assistant: "Juniper")
        #expect(named.first == .assistant(name: "Juniper"))
        #expect(named.contains(.member(MentionDTO(userID: 4, name: "Aiden"))))
        #expect(Self.offered("a", assistant: "Juniper").first == .assistant(name: "Juniper"))
        // Past the token and not its name: gone.
        #expect(!Self.offered("aid", assistant: "Juniper").contains(.assistant(name: "Juniper")))
        #expect(!Self.offered("ai ", assistant: "Juniper").contains(.assistant(name: "Juniper")))
    }

    @Test("not offered at all where `@ai` is not (nil name): the plain member list")
    func notOffered() {
        #expect(Self.offered("", assistant: nil) == Self.roster.dropFirst().map(MentionSuggestion.member))
    }

    @Test("picking it writes `@ai `, never `@<display name>`")
    func pickingWritesTheToken() {
        let assistant = MentionSuggestion.assistant(name: "Juniper")
        #expect(assistant.insertedName == "ai")
        #expect(assistant.shownName == "Juniper")
        #expect(MemberMentions.accept(draft: "Hey @Ju", name: assistant.insertedName) == "Hey @ai ")
        #expect(AssistantMention.mentions(MemberMentions.accept(draft: "@", name: assistant.insertedName)))
    }

    @Test("it is never a member: nothing it inserts resolves to a mention, and its id is no user's")
    func neverAMember() {
        let draft = MemberMentions.accept(draft: "@", name: MentionSuggestion.assistant(name: "Dan").insertedName)
        // Even an assistant that shares a member's display name names nobody.
        #expect(MemberMentions.resolve(body: draft + "what's for dinner?", roster: Self.roster).isEmpty)
        #expect(!Self.roster.contains { $0.userID == MentionSuggestion.assistant(name: "Dan").id })
        #expect(MentionSuggestion.member(Self.roster[1]).insertedName == "Dan")
    }
}
