//
//  NotSentVoiceRow.swift
//  FamilyConnect
//
//  "Voice message not sent · 0:42 [▶] [Send] [✕]" — a recording something
//  other than the person stopped, waiting above the field (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.8). It quotes the reply it was
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

    @State private var asksDelete = false
    /// Its ▶ (Phase 1, S2.7): the file on this device, through `.playback`.
    @State private var player = LocalVoicePlayer()
    @State private var saysAfterRecording = false

    var body: some View {
        HStack(spacing: 10) {
            ZStack {
                Color.appSecondaryFill
                VoiceNotePlayButton(
                    player: player,
                    file: { ParkedRecordings.shared.fileURL(for: entry) },
                    side: Self.tile,
                    saysAfterRecording: $saysAfterRecording)
            }
            .frame(width: Self.tile, height: Self.tile)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

            VStack(alignment: .leading, spacing: 1) {
                Text("Voice message not sent · \(AudioRecorder.timeLabel(entry.duration))")
                    .font(Self.titleFont)
                    .lineLimit(1)
                if let reply = entry.replyTo, let replyAuthor {
                    HStack(spacing: 4) {
                        Text("Replying to \(replyAuthor)")
                            .fontWeight(.semibold)
                            .layoutPriority(1)
                        Text(verbatim: reply.excerpt)
                    }
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                }
                if saysAfterRecording {
                    Text("You can play this after recording.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else if let caption = entry.caption {
                    Text(verbatim: caption)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .accessibilityElement(children: .combine)

            Spacer(minLength: 0)

            Button {
                player.stop()
                onSend()
            } label: {
                Text("Send")
                    // Tap slack toward the 44-point target (S1.1) without
                    // growing the row: its height is part of the composer's.
                    .contentShape(Rectangle().inset(by: -8))
            }
            .controlSize(.small)
            .buttonStyle(.bordered)
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
                Image(systemName: "xmark.circle.fill")
                    .font(Self.glyphFont)
                    .foregroundStyle(.secondary)
                    // Tap slack without a layout change, the staged chip's
                    // idiom: this row's height is part of the composer's.
                    .contentShape(Rectangle().inset(by: -12))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Delete recording")
            .help("Delete recording")
        }
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

    #if os(macOS)
    private static let tile: CGFloat = 32
    private static let glyphFont = Font.body
    private static let titleFont = Font.callout.weight(.medium)
    #else
    private static let tile: CGFloat = 44
    private static let glyphFont = Font.title3
    private static let titleFont = Font.caption.weight(.medium)
    #endif
}
