//
//  MacVoiceRecordingRow.swift
//  FamilyConnect
//
//  What takes the field's place on the Mac while a voice message records
//  (#79, Phase 1 — docs/audio-video-messages-2026-10-04.md, S2.4's desktop
//  column, S2.5, S2.9, S8.3): [Delete] red dot, "0:42", the level meter
//  [Stop] — and the slot beside it is the Send arrow. The meter is the live
//  waveform of the approved design, scrolling in from the trailing edge,
//  and the steady five-bar meter under Reduce Motion.
//
//  IN THE ROW, NEVER ABOVE IT. Phase 0's recording strip was stacked above
//  the input row with a Stop button that was the window's `.defaultAction`,
//  one of the three Return bindings that met there (S8.3). This replaces the
//  paperclip, the sticker, `@ai`, `/draw` and the field, in the same row — so
//  the field is gone while a recording runs, and Return can only reach the
//  slot. Its Stop is an ordinary button; Esc is the composer's.
//
//  The dot, the timer and the meter are the phone's own (VoiceComposerRows):
//  one look for "recording" on both platforms. No hold row, no lock, no
//  "Still recording" line and no Undo row: a Mac has no hold to make them.
//

#if os(macOS)

import SwiftUI

struct MacVoiceRecordingRow: View {
    let elapsed: TimeInterval
    let litBars: Int
    /// The recording's peaks so far: the live waveform scrolling in from the
    /// trailing edge (the five-bar meter under Reduce Motion).
    var peaks: [Float] = []
    /// Started from the paperclip or ⌥⌘R beside words or staged items: no
    /// Stop here, because the slot itself is Stop (S1.3 row 3, S2.4).
    let besideDraft: Bool
    /// 4:30 and later: the timer turns orange and "30 seconds left" takes the
    /// meter's place — words as well as colour (S2.5, WCAG 1.4.1).
    let warning: Bool
    /// The composer's control box: the row is never shorter than the
    /// controls it stands in for.
    let height: CGFloat
    let onDelete: () -> Void
    let onStop: () -> Void

    /// The three widths it can be drawn at, widest first (S2.4: "the level
    /// meter goes first; on desktops the text buttons then become icons with
    /// the same labels").
    enum Fit: CaseIterable {
        case full
        case withoutMeter
        case icons

        var showsMeter: Bool { self == .full }
        var usesWords: Bool { self != .icons }
    }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            row(.full)
            row(.withoutMeter)
            row(.icons)
        }
        .frame(maxWidth: .infinity, minHeight: height, alignment: .leading)
        // One group to VoiceOver, its buttons inside it; the ticking clock
        // is never read (S6).
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Recording a voice message")
        // VoiceOver's escape does what Esc does: Stop, never Delete (S6).
        .accessibilityAction(.escape, onStop)
    }

    /// One width's row. Internal so a test can measure each.
    func row(_ fit: Fit) -> some View {
        HStack(spacing: 8) {
            Button(role: .destructive, action: onDelete) {
                if fit.usesWords {
                    Text("Delete")
                } else {
                    Image(systemName: "trash")
                }
            }
            .controlSize(.small)
            .fixedSize()
            .accessibilityLabel("Delete recording")
            .accessibilityInputLabels([Text("Delete recording"), Text("Delete")])
            .help("Delete recording")

            RecordingDot()
            VoiceTimerText(elapsed: elapsed, warning: warning)
                .fixedSize()
            if warning {
                Text("30 seconds left")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.orange)
                    .lineLimit(1)
            } else if fit.showsMeter {
                VoiceLiveWaveform(peaks: peaks, litBars: litBars, height: 20)
                    .frame(minWidth: 80, maxWidth: 260)
            }

            Spacer(minLength: 0)

            if !besideDraft {
                Button(action: onStop) {
                    if fit.usesWords {
                        Text("Stop")
                    } else {
                        Image(systemName: "stop.circle")
                    }
                }
                .controlSize(.small)
                .fixedSize()
                .accessibilityLabel("Stop recording")
                .accessibilityInputLabels([Text("Stop recording"), Text("Stop")])
                .help("Stop recording")
            }
        }
    }
}

#endif
