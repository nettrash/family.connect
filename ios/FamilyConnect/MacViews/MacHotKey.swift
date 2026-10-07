//
//  MacHotKey.swift
//  FamilyConnect
//
//  ⌃⌥⌘F from any app brings Family Connect forward (#80,
//  docs/mac-menu-bar-2026-10-07.md). Carbon's RegisterEventHotKey: the one
//  system-wide shortcut API that needs neither Accessibility permission nor
//  a hole in the App Sandbox — an NSEvent global monitor needs the first,
//  and sees key presses it has no business seeing.
//
//  It fails, honestly, when another app already holds the combination
//  (eventHotKeyExistsErr), and Settings says so.
//

#if os(macOS)

import AppKit
import Carbon.HIToolbox
import os

@MainActor
final class MacHotKey {

    static let shared = MacHotKey()

    /// ⌃⌥⌘F — F for Family. Shown as such in Settings.
    static let shortcut = "⌃⌥⌘F"
    private static let keyCode = UInt32(kVK_ANSI_F)
    private static let modifiers = UInt32(controlKey | optionKey | cmdKey)
    /// 'FCon': this app's hot keys, so a press is known as ours.
    nonisolated fileprivate static let signature: OSType = 0x4643_6F6E

    private var hotKey: EventHotKeyRef?
    private var handler: EventHandlerRef?

    /// Whether the last attempt to register it failed — another app holds it.
    private(set) var isTaken = false

    /// What a press does; set by MacMenuBar.
    var onPress: (() -> Void)?

    func apply(_ on: Bool) {
        if on { register() } else { unregister() }
    }

    private func register() {
        guard hotKey == nil else { return }
        installHandler()
        var ref: EventHotKeyRef?
        let status = RegisterEventHotKey(
            Self.keyCode, Self.modifiers, EventHotKeyID(signature: Self.signature, id: 1),
            GetEventDispatcherTarget(), 0, &ref)
        isTaken = status != noErr
        if status == noErr {
            hotKey = ref
            AppLog.app.info("The \(Self.shortcut, privacy: .public) shortcut is registered")
        } else {
            AppLog.app.error("The \(Self.shortcut, privacy: .public) shortcut could not be registered: \(status)")
        }
    }

    private func unregister() {
        if let hotKey { UnregisterEventHotKey(hotKey) }
        hotKey = nil
        isTaken = false
    }

    private func installHandler() {
        guard handler == nil else { return }
        var pressed = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        // The event DISPATCHER's target, not the application's: a Cocoa app's
        // run loop hands a hot key press to the dispatcher. With the application
        // target, pressing it did nothing on the owner's Mac (#80); this is the
        // target the established hot-key libraries register with.
        let status = InstallEventHandler(GetEventDispatcherTarget(), macHotKeyPressed, 1, &pressed, nil, &handler)
        if status != noErr {
            AppLog.app.error("The shortcut's handler could not be installed: \(status)")
        }
    }
}

/// Carbon's callback: on the main thread, where the main run loop's event
/// dispatcher calls it. A C function, so it reaches the app through the shared instance.
nonisolated private func macHotKeyPressed(
    _ next: EventHandlerCallRef?, _ event: EventRef?, _ context: UnsafeMutableRawPointer?
) -> OSStatus {
    guard let event else { return OSStatus(eventNotHandledErr) }
    var id = EventHotKeyID()
    let status = GetEventParameter(
        event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil,
        MemoryLayout<EventHotKeyID>.size, nil, &id)
    guard status == noErr, id.signature == MacHotKey.signature else { return OSStatus(eventNotHandledErr) }
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            AppLog.app.info("The shortcut was pressed")
            MacHotKey.shared.onPress?()
        }
    }
    return noErr
}

#endif
