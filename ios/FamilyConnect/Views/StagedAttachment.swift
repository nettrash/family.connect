//
//  StagedAttachment.swift
//  FamilyConnect
//
//  Something prepared and waiting for Send, and the chips that show it.
//
//  Staging separates "what to send" from "when": picking used to send
//  immediately, which meant a caption had to be typed BEFORE choosing the
//  photo — and once picked there was no way out. A message now carries up
//  to TEN attachments (docs/protocol.md, "Photos, videos, audio, files
//  and locations"), so a second pick APPENDS behind the first rather than
//  superseding it; at the cap the composer says so with a brief notice
//  instead of silently dropping the pick.
//
//  Shared, with no `#if os` guard, for the reason LocationAttachmentView is
//  shared: both platforms draw exactly this, from exactly the same
//  prepared item. It moved out of ConversationView when the Mac composer
//  gained a staging step of its own — paste-to-attach must never send by
//  itself, and the Mac had nowhere to put something that was not sent yet.
//

import SwiftUI

/// Media that is prepared and waiting for the user to press Send.
struct StagedAttachment: Identifiable {
    let id = UUID()
    let prepared: MediaPrep.Prepared

    /// How many bytes of THIS item would reach a model, if it were shown to
    /// one — the preview's length where there is a preview, because that is
    /// what the server prefers, and the file's own otherwise
    /// (docs/protocol.md, "Pictures"; `AssistantPictureLimits.wireBytes`).
    ///
    /// Measured ONCE, here, rather than in the composer's notice: that
    /// notice is a computed property of a view body and is re-evaluated on
    /// every keystroke, so asking the file system there would stat the disk
    /// per character typed. Nothing about a staged item changes after it is
    /// staged, so once is also correct.
    let assistantWireBytes: Int

    /// Recorded by this composer's recorder, as opposed to a sound file
    /// picked from disk — which shares `kind=audio` and nothing else. Only a
    /// recording is kept as "not sent" when the person leaves the chat with
    /// it still in review (#79, S2.8): a file can be picked again, a
    /// recording cannot be made again.
    let isVoiceNote: Bool

    init(prepared: MediaPrep.Prepared, isVoiceNote: Bool = false) {
        self.prepared = prepared
        self.isVoiceNote = isVoiceNote
        self.assistantWireBytes = AssistantPictureLimits.wireBytes(
            previewBytes: prepared.previewJPEG?.count,
            originalBytes: Self.fileBytes(at: prepared.fileURL))
    }

    /// The media type this item would travel as: a preview is a JPEG by
    /// definition, so a photo that has one is judged as one.
    var assistantWireMIME: String {
        AssistantPictureLimits.wireMIME(
            mime: prepared.mime, hasPreview: prepared.previewJPEG != nil)
    }

    /// The three facts the family composer's strip reads off a staged item
    /// (`MentionPictureNotice`): what it is, and what it will travel as.
    var assistantPictureCandidate: AssistantPictureCandidate {
        AssistantPictureCandidate(
            kind: prepared.kind, mime: assistantWireMIME, bytes: assistantWireBytes)
    }

    /// `Int.max` when the file cannot be measured, which is the safe
    /// direction: an unmeasurable photograph is described as one that will
    /// not be shown, rather than promised to a model it may never reach.
    private static func fileBytes(at url: URL) -> Int {
        let values = try? url.resourceValues(forKeys: [.fileSizeKey])
        return values?.fileSize ?? Int.max
    }

    /// The protocol's ceiling on one message's attachments
    /// (docs/protocol.md, "Limits": `limits.max_attachments_per_message`).
    /// Both composers refuse the eleventh pick against this, with a
    /// notice — the same number the server would refuse it with.
    ///
    /// An alias, not a second 10: the Share Extension has to cap what it
    /// stages against the same ceiling and cannot see this type, so the
    /// literal lives in ShareHandoff where both targets compile it.
    nonisolated static let maxPerMessage = ShareHandoff.maxAttachmentsPerMessage

    /// May one more item be staged beside `count` already staged?
    ///
    /// A one-line rule, extracted so both composers (and the share-import
    /// path, which arrives with a whole batch) ask the same question and
    /// the tests can pin the answer without a composer on screen.
    static func canAdd(to count: Int) -> Bool {
        count < maxPerMessage
    }

    /// The composer's thumbnail: the same JPEG the bubble will draw.
    /// Files and audio have none — a document is a row, and a sound has
    /// nothing to look at.
    var thumbnail: Image? {
        guard let data = prepared.previewJPEG,
              let image = PlatformImage.decode(data, maxPixels: 240)
        else { return nil }
        return PlatformImage.view(image)
    }

    /// What the chip calls it.
    ///
    /// A word per kind rather than "Photo" for everything that is not a
    /// video, which is what this used to do — a voice note has no name (its
    /// identity is its length), and before the recorder stopped handing one
    /// over it read out the scratch file it was recorded into.
    var label: String {
        if let name = prepared.name, !name.isEmpty { return name }
        switch prepared.kind {
        case AttachmentDTO.Kind.video: return String(localized: "Video")
        case AttachmentDTO.Kind.audio: return String(localized: "Audio")
        case AttachmentDTO.Kind.file: return String(localized: "File")
        default: return String(localized: "Photo")
        }
    }

    /// The glyph drawn where there is no thumbnail.
    var placeholderSymbol: String {
        switch prepared.kind {
        case AttachmentDTO.Kind.audio: "waveform"
        default: "doc"
        }
    }
}

/// A staged item's thumbnail-or-glyph square, shared by the full-width
/// chip (one item staged) and the compact tile (several).
private struct StagedThumbnail: View {
    let item: StagedAttachment
    let side: CGFloat

    var body: some View {
        ZStack {
            if let thumbnail = item.thumbnail {
                thumbnail
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                Color.appSecondaryFill
                Image(systemName: item.placeholderSymbol)
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            if item.prepared.kind == AttachmentDTO.Kind.video {
                Image(systemName: "play.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.white, .black.opacity(0.35))
            }
        }
        // Pinned on BOTH axes, and that is load-bearing: a
        // height-flexible tile beside a Text turns the composer's
        // stack into height distribution and truncates the label.
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

/// The one staged attachment sitting above the field, with the way out —
/// the shape this row has always had, kept for the single-item case. A voice
/// note in review is its own chip, one that plays (#79, S2.7).
struct StagedAttachmentChip: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    var body: some View {
        if item.isVoiceNote {
            StagedVoiceNoteChip(item: item, onRemove: onRemove)
        } else {
            fileChip
        }
    }

    private var fileChip: some View {
        HStack(spacing: 10) {
            StagedThumbnail(item: item, side: 44)

            VStack(alignment: .leading, spacing: 1) {
                Text(item.label)
                    .font(.caption.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text("Add a message, or send it on its own.")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Button {
                onRemove()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    // Tap slack without a layout change: the chip's height
                    // is part of the input bar's, which the thread re-pins
                    // against.
                    .contentShape(Rectangle().inset(by: -12))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
        }
    }
}

/// One staged item among several: a compact tile for the horizontal row,
/// its own remove X riding the corner. The full-width chip's subtitle
/// would repeat per item, so the tile keeps only what identifies the
/// item — the thumbnail and, through accessibility, its label.
struct StagedAttachmentTile: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    var body: some View {
        if item.isVoiceNote {
            StagedVoiceNoteTile(item: item, onRemove: onRemove)
        } else {
            fileTile
        }
    }

    private var fileTile: some View {
        StagedThumbnail(item: item, side: 56)
            .overlay(alignment: .topTrailing) {
                Button {
                    onRemove()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.body)
                        // Two-tone so the X reads on any thumbnail.
                        .foregroundStyle(.white, .black.opacity(0.55))
                        // The glyph stays in the corner; only the tappable
                        // area grows toward the 44pt guideline. The tile
                        // itself has no tap gesture, so the overlap is safe.
                        .frame(width: 28, height: 28, alignment: .topTrailing)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(2)
                .accessibilityLabel("Remove attachment")
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(item.label)
    }
}

/// The staged set above the field: the familiar full-width chip while one
/// item is staged, a horizontally scrolling row of tiles for several.
/// Shared by both composers, so the two cannot drift on the cap or the
/// row's shape.
struct StagedAttachmentRow: View {
    let items: [StagedAttachment]
    let onRemove: (StagedAttachment) -> Void

    var body: some View {
        if items.count == 1, let item = items.first {
            StagedAttachmentChip(item: item) { onRemove(item) }
        } else {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(items) { item in
                        StagedAttachmentTile(item: item) { onRemove(item) }
                    }
                }
                // Room for the X riding above the tile's corner.
                .padding(.top, 2)
            }
        }
    }
}

// MARK: - A voice note in review (#79, S2.7)

/// What a staged voice note needs that a picked file does not: its length,
/// a player for the file on this device, and the rule that deleting ten
/// seconds or more asks first.
extension StagedAttachment {
    /// By the length the recording reported.
    var duration: TimeInterval { Double(prepared.durationMS ?? 0) / 1000 }

    /// Deleting it asks "Delete this recording?" (S1.1, `DELETE_ASKS_FROM_MS`).
    var deleteAsks: Bool {
        (prepared.durationMS ?? 0) >= Int(RecordRules.deleteAsksFromMS)
    }
}

/// The approved design's review chip: "[▶] ▂▅▇▅▃▂ 0:42 [✕]" — a round
/// play button in the tint, the note's own waveform (the recorder's, sent
/// with it) filling in the tint as it plays, its length, and ✕. While it
/// plays the length reads "0:12 / 0:42". ▶ plays the LOCAL file through
/// `.playback` and pauses anything else playing (LocalVoicePlayer); while a
/// recording runs it is dimmed and says "You can play this after
/// recording." (S1.7). ✕ is "Delete recording", and asks from ten seconds.
struct StagedVoiceNoteChip: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    @State private var player = LocalVoicePlayer()
    @State private var asksDelete = false
    @State private var saysAfterRecording = false

    var body: some View {
        HStack(spacing: 10) {
            VoiceNotePlayButton(
                player: player, file: { item.prepared.fileURL }, side: 30,
                saysAfterRecording: $saysAfterRecording)

            Group {
                if saysAfterRecording {
                    Text("You can play this after recording.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    VoiceNoteMiniWaveform(
                        waveform: item.prepared.waveform, player: player, duration: item.duration)
                }
            }
            .accessibilityHidden(true)

            Spacer(minLength: 0)

            Group {
                if player.isPlaying {
                    Text(verbatim: "\(AudioRecorder.timeLabel(player.elapsed)) / \(AudioRecorder.timeLabel(item.duration))")
                } else {
                    Text(verbatim: AudioRecorder.timeLabel(item.duration))
                }
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .fixedSize()
            // What the bars say, in words.
            .accessibilityLabel(Text("Voice message · \(AudioRecorder.timeLabel(item.duration))"))

            Button {
                if item.deleteAsks {
                    asksDelete = true
                } else {
                    player.stop()
                    onRemove()
                }
            } label: {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    // Tap slack without a layout change, as the file chip's.
                    .contentShape(Rectangle().inset(by: -8))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Delete recording")
            // A pointer's tooltip, on the Mac and under an iPad's pointer.
            .help("Delete recording")
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .frame(maxWidth: 360, alignment: .leading)
        .background(.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Color.secondary.opacity(0.25), lineWidth: 1))
        .confirmationDialog("Delete this recording?", isPresented: $asksDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                player.stop()
                onRemove()
            }
            Button("Keep", role: .cancel) {}
        }
        .onDisappear { player.stop() }
    }
}

/// A local voice note's waveform, filling in the tint as `player` plays it:
/// the review chip's and the "not sent" row's.
struct VoiceNoteMiniWaveform: View {
    let waveform: String?
    let player: LocalVoicePlayer
    let duration: TimeInterval
    var height: CGFloat = 22

    /// 48 bars of 3 with gaps of 2.
    static let widest: CGFloat = CGFloat(Waveform.levelCount) * 5 - 2

    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        VoiceWaveformBars(
            levels: Waveform.levelsOrPlaceholder(waveform),
            playedFraction: player.isPlaying || player.elapsed > 0
                ? (UInt64(max(0, player.elapsed) * 1000), UInt64(max(0, duration) * 1000)) : nil,
            played: contrast == .increased ? .primary : .accentColor,
            unplayed: Color.secondary.opacity(contrast == .increased ? 0.7 : 0.45))
            .frame(height: height)
            // The 48 bars' own width at most, so a wide row does not leave
            // its length and buttons stranded far from the bars.
            .frame(minWidth: 0, maxWidth: Self.widest)
    }
}

/// A voice note among several staged items: the tile plays it.
struct StagedVoiceNoteTile: View {
    let item: StagedAttachment
    let onRemove: () -> Void

    @State private var player = LocalVoicePlayer()
    @State private var asksDelete = false
    @State private var saysAfterRecording = false

    var body: some View {
        ZStack {
            Color.appSecondaryFill
            VoiceNoteMiniWaveform(
                waveform: item.prepared.waveform, player: player, duration: item.duration, height: 14)
                .padding(.horizontal, 6)
                .frame(maxHeight: .infinity, alignment: .bottom)
                .padding(.bottom, 5)
                .accessibilityHidden(true)
            VoiceNotePlayButton(
                player: player, file: { item.prepared.fileURL }, side: 30,
                saysAfterRecording: $saysAfterRecording)
                .padding(.bottom, 12)
        }
        .frame(width: 56, height: 56)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(alignment: .topTrailing) {
            Button {
                if item.deleteAsks {
                    asksDelete = true
                } else {
                    player.stop()
                    onRemove()
                }
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.body)
                    .foregroundStyle(.white, .black.opacity(0.55))
                    .frame(width: 28, height: 28, alignment: .topTrailing)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(2)
            .accessibilityLabel("Delete recording")
            .help("Delete recording")
        }
        .confirmationDialog("Delete this recording?", isPresented: $asksDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                player.stop()
                onRemove()
            }
            Button("Keep", role: .cancel) {}
        }
        .onDisappear { player.stop() }
    }
}

/// ▶ / ❚❚ over a local voice note: a disc in the tint, `side` across, its
/// target grown toward 44 without growing the row — dimmed while a recording
/// runs anywhere in the app, saying why instead of playing (S1.7). Shared by
/// the review chip, its tile and the "not sent" row.
struct VoiceNotePlayButton: View {
    let player: LocalVoicePlayer
    /// Where the file is — asked at the tap, never per redraw: the composer
    /// redraws on every keystroke, and finding a parked file reads the disk.
    let file: () -> URL?
    let side: CGFloat
    @Binding var saysAfterRecording: Bool

    private var recording: Bool { VoiceRecordingArbiter.shared.isRecording }

    var body: some View {
        Button {
            guard !recording else {
                saysAfterRecording = true
                AccessibilityNotification.Announcement(String(localized: "You can play this after recording.")).post()
                return
            }
            saysAfterRecording = false
            guard let url = player.url ?? file() else { return }
            player.toggle(url)
        } label: {
            ZStack {
                Circle().fill(Color.accentColor)
                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: side * 0.4, weight: .semibold))
                    .foregroundStyle(.white)
                    .offset(x: player.isPlaying ? 0 : side * 0.04)
            }
            .frame(width: side, height: side)
            .opacity(recording ? 0.35 : 1)
            .contentShape(Circle().inset(by: -max(0, (44 - side) / 2)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(player.isPlaying ? "Pause" : "Play")
        // Two Texts, not a ternary of literals: that would be a `String`,
        // which `.help` shows verbatim and never translates.
        .help(player.isPlaying ? Text("Pause") : Text("Play"))
        .accessibilityValue(recording ? Text("You can play this after recording.") : Text(""))
        // A recording starting pauses it through NowPlaying; once it stops
        // the sentence has nothing left to say.
        .onChange(of: recording) { _, now in
            if !now { saysAfterRecording = false }
        }
    }
}
