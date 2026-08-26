//
//  MacFilePicker.swift
//  FamilyConnect
//
//  One open panel, for every kind of attachment.
//
//  On the Mac there is no reason to split "photo or video" from "file" the
//  way the phone does: what people attach lives in the file system, and
//  one panel reaches all of it. The KIND is then decided from the file
//  itself (MediaPrep.prepareAny), not from which button was pressed.
//

#if os(macOS)

import AppKit
import UniformTypeIdentifiers

enum MacFilePicker {
    /// The file the user chose, or nil if they cancelled.
    ///
    /// Runs modally on purpose: it is a direct answer to a click, and the
    /// alternative (a sheet with a completion handler) would leave the
    /// caller juggling state for something that takes a second.
    static func pickOne() -> URL? {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        // Anything at all — the protocol's file kind accepts any type, and
        // telling a family what they may send is exactly what it avoids.
        panel.allowedContentTypes = []
        // "Attach", not "Send": the panel puts the file in the composer as
        // a staged chip, so a caption can be typed before anything goes.
        panel.prompt = String(localized: "Attach")
        return panel.runModal() == .OK ? panel.url : nil
    }
}

#endif
