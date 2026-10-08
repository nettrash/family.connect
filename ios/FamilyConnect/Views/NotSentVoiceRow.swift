//
//  NotSentVoiceRow.swift
//  FamilyConnect
//
//  "Not sent [▶] ▂▅▇▅▃ 0:42 Send ✕" — a recording something other than the
//  person stopped, waiting above the field (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.8; the approved design's
//  not-sent chip, bordered in the warning colour). VoiceOver still reads it
//  as "Voice message not sent · 0:42". It quotes the reply it was
//  recorded under and shows its caption, if it has them; its Send sends THAT
//  note with THAT reply and caption and nothing else, and ✕ deletes it —
//  asking first at ten seconds or more. Its ▶ (Phase 1) plays the file on
//  this device through `.playback`, in the tile at its leading edge, and is
//  dimmed while a recording runs.
//
//  Shared, with no `#if os` guard around the type, for StagedAttachment's
//  reason: both composers draw exactly this, from exactly the same entry.
//

import SwiftUI

struct NotSentVoiceRow: View {
    let entry: ParkedRecordings.Entry
    /// Who wrote the quoted message, named by the composer that holds the
    /// roster. Nil when the note answers nothing.
    let replyAuthor: String?
    let onSend: () -> Void
    let onDelete: () -> Void

    @Environment(\.dynamicTypeSize) private var typeSize

    @State private var asksDelete = false
    /// Its ▶ (Phase 1, S2.7): the file on this device, through `.playback`.
    @State private var player = LocalVoicePlayer()
    @State private var saysAfterRecording = false

    /// The reply and the caption, on one line under the waveform.
    private var secondLine: String? {
        if saysAfterRecording { return String(localized: "You can play this after recording.") }
        var parts: [String] = []
        if let reply = entry.replyTo, let replyAuthor {
            parts.append("\(String(localized: "Replying to \(replyAuthor)")) \(reply.excerpt)")
        }
        if let caption = entry.caption { parts.append(caption) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 8) {
            notSentMark

            VoiceNotePlayButton(
                player: player,
                file: { ParkedRecordings.shared.fileURL(for: entry) },
                side: Self.disc,
                saysAfterRecording: $saysAfterRecording)

            VStack(alignment: .leading, spacing: 1) {
                VoiceNoteMiniWaveform(
                    waveform: entry.waveform, player: player, duration: entry.duration,
                    height: secondLine == nil ? 22 : 16)
                    .accessibilityHidden(true)
                if let secondLine {
                    Text(verbatim: secondLine)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            // The row in words: "Voice message not sent · 0:42", then the
            // reply and caption it carries.
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text("Voice message not sent · \(AudioRecorder.timeLabel(entry.duration))"))

            Spacer(minLength: 0)

            Text(verbatim: AudioRecorder.timeLabel(player.isPlaying ? player.elapsed : entry.duration))
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .fixedSize()
                .accessibilityHidden(true)

            Button {
                player.stop()
                onSend()
            } label: {
                Text("Send")
                    .font(Self.sendFont)
                    .foregroundStyle(.tint)
                    // Tap slack toward the 44-point target (S1.1) without
                    // growing the row: its height is part of the composer's.
                    .contentShape(Rectangle().inset(by: -10))
            }
            .buttonStyle(.plain)
            .fixedSize()
            .accessibilityLabel("Send voice message")
            .help("Send voice message")

            Button {
                if entry.deleteAsks {
                    asksDelete = true
                } else {
                    player.stop()
                    onDelete()
                }
            } label: {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
                    // Tap slack without a layout change, the staged chip's
                    // idiom: this row's height is part of the composer's.
                    .contentShape(Rectangle().inset(by: -10))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Delete recording")
            .help("Delete recording")
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 10)
        // Its words grow no further than this: past it, "Not sent", the
        // length, Send and the fixed ✕ — none of which can shrink — would
        // push the waveform out and Delete off the row's end. The row is
        // the composer's chrome; VoiceOver reads it in full at any size.
        .dynamicTypeSize(...Self.largestType)
        .background(.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        // The warning colour, softly: this one is waiting on a decision.
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Color.orange.opacity(0.45), lineWidth: 1))
        // A recording cannot be made again: ten seconds or more asks (S1.1),
        // and Keep is the answer that loses nothing.
        .confirmationDialog("Delete this recording?", isPresented: $asksDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                player.stop()
                onDelete()
            }
            Button("Keep", role: .cancel) {}
        }
        .onDisappear { player.stop() }
    }

    /// "Not sent" in the warning colour — at the accessibility sizes the
    /// warning glyph in its place, which leaves the waveform its room. The
    /// orange border says the same, and the row's label says it in words.
    @ViewBuilder
    private var notSentMark: some View {
        if typeSize.isAccessibilitySize {
            Image(systemName: "exclamationmark.circle.fill")
                .font(Self.labelFont)
                .foregroundStyle(.orange)
                .accessibilityHidden(true)
        } else {
            Text("Not sent")
                .font(Self.labelFont)
                .foregroundStyle(.orange)
                .lineLimit(1)
                .fixedSize()
                .accessibilityHidden(true)
        }
    }

    /// The largest text the row draws at (see the body).
    static let largestType = DynamicTypeSize.accessibility2

    #if os(macOS)
    private static let disc: CGFloat = 26
    private static let labelFont = Font.callout.weight(.semibold)
    private static let sendFont = Font.callout.weight(.semibold)
    #else
    private static let disc: CGFloat = 30
    private static let labelFont = Font.caption.weight(.semibold)
    private static let sendFont = Font.subheadline.weight(.semibold)
    #endif
}
