//
//  AttachMenu.swift
//  FamilyConnect
//
//  What the composer's paperclip offers, in what order and in which group —
//  issue #78, docs/attachment-menu-2026-10-07.md. One rule for the iPhone,
//  the iPad and the Mac, so the two composers render the same list instead
//  of each keeping its own, and a test can pin it without a composer on
//  screen.
//
//  The spec, read top to bottom on every client:
//
//      Photo or Video    ("Show the Assistant a Photo…" INSTEAD, in the assistant chat)
//      Camera            (iPhone/iPad only, where there is one)
//      File
//      Paste
//      ──────────────
//      Record Voice Message
//      Record Video Message
//      ──────────────
//      Ask the Assistant (the family chat, where the assistant is offered — #106)
//      Location
//      Poll              (the family chat only)
//
//  "Ask the Assistant" is the `@ai` button that used to stand in the row
//  (#106, docs/composer-and-wide-layout-2026-10-20.md): the row gave the
//  field its width back, and the button became the first line of the last
//  group here and the first entry of the `@` list. It types `@ai ` as the
//  button did.
//
//  This decides WHICH items appear. Whether one that appears is enabled
//  (a call in progress, an unsent voice message, a busy composer) stays
//  with the composer, which owns those facts.
//

nonisolated enum AttachMenu {

    enum Item: Hashable, CaseIterable {
        /// The system photo/video picker.
        case photoOrVideo
        /// The assistant chat's images-only picker, IN PLACE OF `photoOrVideo`.
        case assistantPhoto
        /// The system camera — iPhone/iPad only.
        case camera
        case file
        case paste
        case voiceMessage
        case videoMessage
        /// Types `@ai ` into the draft — the family chat only (#106).
        case askAssistant
        case location
        case poll
    }

    /// The menu, as groups drawn with a separator between them. A group
    /// that would be empty is left out, so no separator is ever drawn
    /// around nothing (the assistant chat has no recording group).
    ///
    /// - Parameters:
    ///   - isAssistantChat: the assistant's own chat.
    ///   - showsPictureAttach: `AssistantSurfaces.offersPictureAttach` — both
    ///     vision locks open (only ever true in the assistant chat).
    ///   - hasCamera: a system camera to hand off to. Always false on the Mac.
    ///   - offersVoice: voice messages are offered in this chat.
    ///   - roundAvailable: round video is offered in this chat. Only read
    ///     alongside `offersVoice` — the video item sits in the recording
    ///     group, right below the voice item.
    ///   - isFamilyOrGroup: the family chat or a group (`ChatRules`), the
    ///     only places a poll is valid.
    ///   - offersAssistant: the server advertises an assistant this client
    ///     offers (one with a named `processor`) — exactly where `@ai` may
    ///     be written. Read only in the family chat or a group: a direct
    ///     chat is two people who each already have a private assistant,
    ///     and the assistant's own chat answers everything without being
    ///     asked.
    static func groups(
        isAssistantChat: Bool,
        showsPictureAttach: Bool,
        hasCamera: Bool,
        offersVoice: Bool,
        roundAvailable: Bool,
        isFamilyOrGroup: Bool,
        offersAssistant: Bool
    ) -> [[Item]] {
        // In the assistant's chat the picture doors ARE the vision gate,
        // and absent rather than disabled when it is shut (protocol.md,
        // "Pictures"). Everywhere else they are unconditional.
        let picturesAllowed = !isAssistantChat || showsPictureAttach

        var sources: [Item] = []
        if isAssistantChat {
            if showsPictureAttach { sources.append(.assistantPhoto) }
        } else {
            sources.append(.photoOrVideo)
        }
        if hasCamera, picturesAllowed { sources.append(.camera) }
        sources += [.file, .paste]

        var recording: [Item] = []
        if offersVoice {
            recording.append(.voiceMessage)
            if roundAvailable { recording.append(.videoMessage) }
        }

        var sharing: [Item] = []
        if isFamilyOrGroup, offersAssistant { sharing.append(.askAssistant) }
        sharing.append(.location)
        if isFamilyOrGroup { sharing.append(.poll) }

        return [sources, recording, sharing].filter { !$0.isEmpty }
    }
}
