//
//  DroppedAttachment.swift
//  FamilyConnect
//
//  What a drag dropped on a conversation MEANS, before anything is read
//  from disk.
//
//  Drag and drop is not a protocol feature: a dropped file becomes an
//  ordinary attachment upload followed by the existing claim-on-send
//  (docs/protocol.md, "Photos, videos and files"), and a dropped link
//  becomes words in the draft. Nothing new goes over the wire. What this
//  file owns is the one decision a drop forces that a picker never does:
//  an NSOpenPanel is CONFIGURED to hand back only files that can be sent
//  (MacFilePicker sets `canChooseDirectories = false`), while a drag hands
//  over whatever the person was holding — a folder, an .app bundle, a link
//  off a web page, several of those at once.
//
//  Shared rather than sitting in MacViews, for the reason
//  `StagedAttachment.canAdd` is shared: it is a pure rule over URLs with no
//  window behind it, and putting it here is what lets the tests pin the
//  answer without a composer on screen. Only the Mac has a drop target
//  today (iOS has no equivalent gesture into the composer), so nothing on
//  the phone calls this yet — the rule is still the same rule if it ever
//  does.
//
//  Preparation itself is NOT here. Everything a drop accepts goes through
//  MediaPrep by way of the composer's one ingestion path, so a dropped
//  photo is downscaled exactly like a picked one, a dropped animation is
//  still sent as a file rather than flattened, and the single 100 MB
//  ceiling and the ten-per-message cap are enforced where they always were.
//
//  Android counterpart: none. Android's composer has no drag target.
//

import Foundation

nonisolated enum DroppedAttachment {

    /// What a drop is, as far as a composer is concerned.
    ///
    /// Three answers rather than two, for the reason `ClipboardAttachment`
    /// has three: "files to attach" and "a link to type" want different
    /// things from the composer, and "nothing I can use" has to be
    /// distinguishable from both so the drop can be REFUSED — a refused
    /// drop springs back to where it came from, which is the only feedback
    /// a person gets that a folder was never going to be sendable.
    enum Decision: Equatable {
        /// Files worth preparing, in the order they were dropped.
        case attach([URL])
        /// Links, which belong in the draft as words.
        case link([URL])
        /// Nothing this composer can do anything with.
        case nothing
    }

    /// THE rule: what a drop of these URLs means.
    ///
    /// FILES WIN, which is the mirror image of the clipboard's "words win"
    /// (`ClipboardAttachment.decide`) and settles the case that decides it:
    /// an image dragged out of a browser offers its `http(s)` source URL,
    /// and a file dragged out of Finder offers a `file:` URL — sometimes
    /// alongside a text representation of its own path. Taking the link
    /// when there is a real file present would make the feature useless
    /// exactly where it is most wanted.
    ///
    /// A DIRECTORY is neither, and refusing it is not fussiness:
    /// `MediaPrep.prepareFile` would happily copy a folder and stage it as
    /// a `kind=file` whose upload has no bytes, and an .app or an .rtfd is
    /// a folder too. `isDirectory` is passed in rather than read here so
    /// the rule stays free of the file system — the caller does the one
    /// piece of IO, exactly where the sandbox has granted it.
    ///
    /// - Parameter isDirectory: whether a file URL points at a directory.
    ///   Answering `true` for anything unreadable is deliberate on the
    ///   caller's side: a URL that cannot even be probed is one `prepare`
    ///   could not have read either.
    static func decide(_ urls: [URL], isDirectory: (URL) -> Bool) -> Decision {
        let files = urls.filter { $0.isFileURL && !isDirectory($0) }
        if !files.isEmpty { return .attach(files) }
        let links = urls.filter { !$0.isFileURL }
        if !links.isEmpty { return .link(links) }
        return .nothing
    }
}
