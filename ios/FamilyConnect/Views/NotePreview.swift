//
//  NotePreview.swift
//  FamilyConnect
//
//  The sticker as it will look, drawn inside the editor.
//
//  The board's text FITS its note (docs/protocol.md, "Board"), which makes
//  the size step a choice with a visible result: at `small` a long note is
//  small type, at `large` it is big. The protocol asks for exactly that to
//  be in front of the author while they write, rather than discovered on
//  the wall afterwards — and it is the answer to "the text should have an
//  impact on the note size" that does not take the size away from the
//  author or move everybody else's notes around.
//
//  It draws through the same NoteSize the wall draws through, so the two
//  cannot drift: whatever the sticker does with the type here is what it
//  will do out there.
//
//  Android counterpart: NotePreview in ui/board/BoardScreen.kt.
//

import SwiftUI

/// The longest a note may be, from the protocol: "text is trimmed,
/// non-empty and at most 280 characters".
///
/// Enforced where the author is TYPING, so the refusal is a full field
/// rather than a save that comes back `validation` — which on Android used
/// to be swallowed entirely, and on Apple was an alert about nothing the
/// person could see.
enum NoteText {
    static let maxLength = 280

    /// What may be kept of `text`: the first 280 CHARACTERS, counted the
    /// way the server counts them.
    ///
    /// The server's cap is `text.chars().count()` — Unicode scalars — and
    /// Swift's `String.count` is grapheme clusters, which is a smaller
    /// number for a family emoji or an accented letter typed as a base plus
    /// a combining mark. Counting scalars here means the two agree on the
    /// one string where they could differ, and a note that looks under the
    /// cap is never refused by the server.
    static func capped(_ text: String) -> String {
        let scalars = text.unicodeScalars
        guard scalars.count > maxLength else { return text }
        let end = scalars.index(scalars.startIndex, offsetBy: maxLength)
        return String(String.UnicodeScalarView(scalars[scalars.startIndex..<end]))
    }

    /// How many more characters the author may type. Never negative.
    static func remaining(_ text: String) -> Int {
        max(0, maxLength - text.unicodeScalars.count)
    }

    /// Show the counter only when it starts to matter, so an ordinary note
    /// is written without a number counting down at it.
    static func shouldShowCounter(_ text: String) -> Bool {
        remaining(text) <= 40
    }

    /// The longest an event's place may be: "a `place` of at most 200
    /// characters" — scalars again, as the server counts them.
    static let maxPlaceLength = 200

    /// The longest one line of a task list may be, and the most lines one
    /// list may hold — the server's own numbers, so a client never lets
    /// somebody write a list whose save fails for a reason nobody can see
    /// (docs/protocol.md, "Board").
    static let maxTaskItemLength = 100
    static let maxTaskItems = 20

    /// `text` cut to `limit` scalars. `capped(_:)` is this at the note's own
    /// limit; a place's is 200. Counting graphemes here would let a place
    /// that looks under the limit be refused by the server.
    static func capped(_ text: String, to limit: Int) -> String {
        let scalars = text.unicodeScalars
        guard scalars.count > limit else { return text }
        let end = scalars.index(scalars.startIndex, offsetBy: limit)
        return String(String.UnicodeScalarView(scalars[scalars.startIndex..<end]))
    }
}

/// The sticker, at the size and colour the author has chosen, with the text
/// fitted exactly as the wall fits it.
struct NotePreview: View {
    let text: String
    let color: String
    let size: NoteSize
    let font: NoteFont

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(text)
                .font(font.font(for: size))
                .foregroundStyle(.black.opacity(0.85))
                .lineLimit(size.fittedLineLimit)
                .minimumScaleFactor(size.minimumTextScale)
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(width: size.frame.width, height: size.frame.height, alignment: .topLeading)
        .background(NoteColor.swiftUI(color), in: RoundedRectangle(cornerRadius: 10))
        .shadow(color: .black.opacity(0.12), radius: 3, y: 2)
        // One element to VoiceOver, and never a second reading of the text
        // the field above already holds: what this adds is the LOOK.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Preview"))
        .accessibilityValue(Text(size.title))
    }
}

/// The picture on a photo note, filling the sticker's width.
///
/// Through the same AttachmentStore a message's photo comes from, by id:
/// the bytes are cached once per device and a board that fetched its own
/// copies would double the storage for the same pixels. The PREVIEW is what
/// a sticker wants — a 220pt tile has no use for 1600 pixels, and the
/// preview is what arrives first on a slow connection.
struct NotePicture: View {
    let attachmentID: Int64
    /// How tall the picture is drawn — see `height(cardHeight:hasCaption:)`.
    var height: CGFloat = 84

    @Environment(AttachmentStore.self) private var store

    /// 84 where the card has room for it, and less where it does not.
    ///
    /// A fixed 84 was right for the phone's medium and large stickers and
    /// wrong for every smaller card: the Mac's small and medium cards are 88
    /// and 110 tall inside 10 of padding, so 84 of picture pushed the caption
    /// and the author line out through the bottom of the card. What is left
    /// after the padding, the author line and — when there is one — a line
    /// of caption, never below a strip that still reads as a picture.
    static func height(cardHeight: CGFloat, hasCaption: Bool) -> CGFloat {
        let room = cardHeight - 20 - 16 - (hasCaption ? 22 : 0)
        return min(84, max(24, room))
    }

    /// READING `store.generation` HERE IS LOAD-BEARING, exactly as it is in
    /// AttachmentView: the store's caches are `@ObservationIgnored`, so
    /// `generation` is the only thing a view can depend on, and a view that
    /// never touches it never redraws when its fetch lands. That is issue
    /// #69: every picture pinned on another device — and the pinner's own,
    /// on a Mac board window left open — stayed a spinner for good.
    ///
    /// The preview first, the photo itself once the server has said there is
    /// no preview (`AttachmentStore.previewOrPhoto`) — a pin whose preview
    /// upload was lost still has its picture.
    private var image: Image? {
        _ = store.generation
        return store.previewOrPhoto(id: attachmentID)
    }

    var body: some View {
        Group {
            if let image {
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                // The shape the picture will take, so the caption does not
                // jump when the bytes land.
                Rectangle()
                    .fill(.black.opacity(0.06))
                    .overlay {
                        ProgressView().controlSize(.small)
                    }
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: height)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        // The picture is the note; VoiceOver hears the caption below it and
        // the note's own combined label, so a second announcement here
        // would say everything twice.
        .accessibilityHidden(true)
    }
}
