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

    /// The files the user chose, or [] if they cancelled. The many-item
    /// door for a message that now carries up to ten attachments; the
    /// composer enforces the cap, where the notice can be shown.
    static func pickMany() -> [URL] {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = []
        panel.prompt = String(localized: "Attach")
        return panel.runModal() == .OK ? panel.urls : []
    }

    /// Pictures only, for the one place on this app where the KIND is the
    /// whole point rather than an inference: showing the assistant a
    /// photograph (docs/protocol.md, "Pictures").
    ///
    /// The Mac's answer to the phone's photo picker, and it is a better
    /// one than a general panel with a warning afterwards: a video, a
    /// document and a sound never reach a model at any size, under any
    /// setting, so a panel that offered them here would be offering
    /// something that cannot happen. `allowedContentTypes` is the place to
    /// say so — it greys out what will not be looked at, in the panel,
    /// before anything is chosen.
    ///
    /// `.image` rather than the two MIME types that actually travel: every
    /// photo this app uploads is re-encoded to JPEG on its way out
    /// (MediaPrep is the only door), so a HEIC picked here arrives at the
    /// server as a JPEG and is shown to the model like any other. Naming
    /// JPEG and PNG in the panel would refuse the format an iPhone's
    /// library is actually full of, for a limit this platform cannot hit.
    ///
    /// Capped at four in the panel itself — the number the model is shown —
    /// so the limit is a thing the Mac refuses rather than a sentence
    /// somebody reads afterwards. Multi-select stays on: choosing four at
    /// once is the normal case, and the composer's own ten-per-message cap
    /// still applies underneath.
    static func pickPictures(limit: Int = AssistantPictureLimits.maxPerQuestion) -> [URL] {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = [.image]
        panel.prompt = String(localized: "Attach")
        panel.message = String(localized: "These photos go to the model your server is set up to use.")
        return panel.runModal() == .OK ? Array(panel.urls.prefix(limit)) : []
    }
}

#endif
