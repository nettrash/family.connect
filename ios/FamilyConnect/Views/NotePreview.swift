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

    @Environment(AttachmentStore.self) private var store

    var body: some View {
        Group {
            if let image = store.image(id: attachmentID, preview: true, mayArriveLate: true) {
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
        .frame(height: 84)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        // The picture is the note; VoiceOver hears the caption below it and
        // the note's own combined label, so a second announcement here
        // would say everything twice.
        .accessibilityHidden(true)
    }
}
