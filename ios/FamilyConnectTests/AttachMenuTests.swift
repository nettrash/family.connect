//
//  AttachMenuTests.swift
//  FamilyConnectTests
//
//  One attachment menu on every client (#78, docs/attachment-menu-2026-10-07.md):
//  the items both Apple composers render, their order, and their groups.
//

import Testing
import UniformTypeIdentifiers
@testable import FamilyConnect

struct AttachMenuTests {

    private typealias I = AttachMenu.Item

    /// A family or direct chat on the phone, everything offered. The flags a
    /// test does not name are the ones a real chat of that kind has.
    private static func phone(
        isAssistantChat: Bool = false,
        showsPictureAttach: Bool = false,
        hasCamera: Bool = true,
        offersVoice: Bool = true,
        roundAvailable: Bool = true,
        isFamilyChat: Bool = false
    ) -> [[I]] {
        AttachMenu.groups(
            isAssistantChat: isAssistantChat, showsPictureAttach: showsPictureAttach,
            hasCamera: hasCamera, offersVoice: offersVoice,
            roundAvailable: roundAvailable, isFamilyChat: isFamilyChat)
    }

    @Test("the family chat on the phone: the spec's full menu, in three groups")
    func familyChatPhone() {
        #expect(Self.phone(isFamilyChat: true) == [
            [.photoOrVideo, .camera, .file, .paste],
            [.voiceMessage, .videoMessage],
            [.location, .poll],
        ])
    }

    @Test("a direct chat has no poll")
    func directChatPhone() {
        #expect(Self.phone() == [
            [.photoOrVideo, .camera, .file, .paste],
            [.voiceMessage, .videoMessage],
            [.location],
        ])
    }

    @Test("the Mac is the phone's list without the camera")
    func macHasNoCamera() {
        #expect(Self.phone(hasCamera: false, isFamilyChat: true) == [
            [.photoOrVideo, .file, .paste],
            [.voiceMessage, .videoMessage],
            [.location, .poll],
        ])
    }

    @Test("the assistant chat with pictures allowed: its own photo door INSTEAD, never both; no recording group")
    func assistantChatWithVision() {
        let groups = Self.phone(
            isAssistantChat: true, showsPictureAttach: true, offersVoice: false, roundAvailable: false)
        #expect(groups == [
            [.assistantPhoto, .camera, .file, .paste],
            [.location],
        ])
        #expect(!groups.joined().contains(.photoOrVideo))
    }

    @Test("the assistant chat with pictures shut: no picture door and no camera at all")
    func assistantChatWithoutVision() {
        #expect(Self.phone(
            isAssistantChat: true, showsPictureAttach: false, offersVoice: false, roundAvailable: false
        ) == [
            [.file, .paste],
            [.location],
        ])
    }

    @Test("the assistant chat on the Mac")
    func assistantChatMac() {
        #expect(Self.phone(
            isAssistantChat: true, showsPictureAttach: true, hasCamera: false,
            offersVoice: false, roundAvailable: false
        ) == [
            [.assistantPhoto, .file, .paste],
            [.location],
        ])
    }

    @Test("no camera on the device: the item is absent, not the group")
    func noCamera() {
        #expect(Self.phone(hasCamera: false)[0] == [.photoOrVideo, .file, .paste])
    }

    @Test("round video only beside voice: without voice there is no recording group at all")
    func recordingGroup() {
        #expect(Self.phone(roundAvailable: false)[1] == [.voiceMessage])
        #expect(Self.phone(offersVoice: false, roundAvailable: true) == [
            [.photoOrVideo, .camera, .file, .paste],
            [.location],
        ])
    }

    @Test("no group is ever empty, and no item appears twice", arguments: 0..<64)
    func neverAnEmptyGroup(bits: Int) {
        let groups = AttachMenu.groups(
            isAssistantChat: bits & 1 != 0, showsPictureAttach: bits & 2 != 0,
            hasCamera: bits & 4 != 0, offersVoice: bits & 8 != 0,
            roundAvailable: bits & 16 != 0, isFamilyChat: bits & 32 != 0)
        #expect(groups.allSatisfy { !$0.isEmpty })
        let items = Array(groups.joined())
        #expect(Set(items).count == items.count)
        #expect(!(items.contains(.photoOrVideo) && items.contains(.assistantPhoto)))
        // The spec's order holds whatever is left out.
        let order = I.allCases
        let positions = items.compactMap { order.firstIndex(of: $0) }
        #expect(positions == positions.sorted())
    }

    @Test("a GIF, WebP or BMP off the photo picker goes as the file it is — except to the assistant (PR #87)")
    func animatedPicksKeepTheirBytes() {
        #expect(PickedMediaPrep.keptAsFile([.gif], keepsAnimated: true) == .gif)
        #expect(PickedMediaPrep.keptAsFile([.webP, .image], keepsAnimated: true) == .webP)
        #expect(PickedMediaPrep.keptAsFile([.bmp], keepsAnimated: true) == .bmp)
        #expect(PickedMediaPrep.keptAsFile([.heic, .jpeg], keepsAnimated: true) == nil)
        #expect(PickedMediaPrep.keptAsFile([.png], keepsAnimated: true) == nil)
        // The assistant is shown a photo: there, a GIF is its still.
        #expect(PickedMediaPrep.keptAsFile([.gif], keepsAnimated: false) == nil)
    }
}
