//
//  VoiceComposerRows.swift
//  FamilyConnect
//
//  What takes the field's place while a voice message is being made (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.4, S2.9): the recording row.
//  (The hold row, "Slide to cancel", the lock pill and the Undo row went with
//  the hold on 2026-10-06: the microphone only taps.)
//
//  IN THE ROW, NEVER ABOVE IT. The recording strip used to be stacked above
//  the input row, which grew the bar — and the bar's height must stay
//  constant, or the newest messages slide under it (ConversationView's
//  header). Each of these is drawn in the space the controls or the field
//  already occupy, at the same height, so nothing above them moves.
//
//  The red dot pulses once a second (steady under Reduce Motion); the timer
//  is m:ss in monospaced digits from the recorder's own clock; the recording
//  row draws the LIVE waveform of the peaks so far (VoiceLiveWaveform), and
//  under Reduce Motion the steady level meter — five bars lit at −50, −40,
//  −30, −20 and −10 dBFS of the PEAK, the measure every client shares
//  (`AudioRecorder.litBars`). The clock is never announced — only state
//  changes are (S6). The look is the approved design "Voice and Video
//  Messages": a capsule row.
//
//  The dot, the meter and the timer are shared: the Mac's recording row
//  (MacVoiceRecordingRow) draws the same three, so the two platforms cannot
//  drift on what "recording" looks like (S2.9). The row itself is the
//  phone's and the tablet's — a Mac has its own, and no haptics.
//

import SwiftUI

/// The 8-point red dot, pulsing between 100 % and 40 % once a second.
struct RecordingDot: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dim = false

    var body: some View {
        Circle()
            .fill(Color.red)
            .frame(width: 8, height: 8)
            .opacity(reduceMotion ? 1 : (dim ? 0.4 : 1))
            .animation(
                reduceMotion ? nil : .easeInOut(duration: 0.5).repeatForever(autoreverses: true),
                value: dim)
            .onAppear { dim = true }
            .accessibilityHidden(true)
    }
}

/// Five bars, 3 × 16 points each, lit by the peak level (S2.9).
struct VoiceLevelMeter: View {
    let lit: Int

    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<AudioRecorder.meterSteps.count, id: \.self) { index in
                RoundedRectangle(cornerRadius: 1, style: .continuous)
                    .fill(index < lit ? AnyShapeStyle(.primary) : AnyShapeStyle(.quaternary))
                    .frame(width: 3, height: 16)
            }
        }
        .accessibilityHidden(true)
    }
}

/// The timer: orange from 4:30, beside "30 seconds left" — words as well as
/// colour (WCAG 1.4.1).
struct VoiceTimerText: View {
    let elapsed: TimeInterval
    let warning: Bool

    var body: some View {
        Text(verbatim: AudioRecorder.timeLabel(elapsed))
            .font(.callout.monospacedDigit())
            .foregroundStyle(warning ? Color.orange : Color.primary)
            .accessibilityHidden(true)
    }
}

#if os(iOS)

/// Hands-free: [trash] red dot, timer, the LIVE waveform [stop] — and the
/// slot beside it is the Send arrow. Beside words or staged items there is
/// no Stop here: the slot itself is Stop (S2.4).
struct VoiceRecordingRow: View {
    let elapsed: TimeInterval
    let litBars: Int
    /// The recording's peaks so far — the waveform scrolling in from the
    /// trailing edge (the five-bar meter under Reduce Motion).
    var peaks: [Float] = []
    let besideDraft: Bool
    let warning: Bool
    let control: CGFloat
    let onDelete: () -> Void
    let onStop: () -> Void
    let onMagicTap: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .font(.system(size: 18))
                    .foregroundStyle(.secondary)
                    .frame(width: control, height: control)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Delete recording")

            HStack(spacing: 8) {
                RecordingDot()
                VoiceTimerText(elapsed: elapsed, warning: warning)
                    .fixedSize()
                middle
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if !besideDraft {
                Button(action: onStop) {
                    Image(systemName: "stop.fill")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: control, height: control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Stop recording")
            }
        }
        .frame(maxWidth: .infinity, minHeight: control, maxHeight: control)
        .background(.fill.tertiary, in: Capsule())
        // Wherever VoiceOver's focus is inside the row, Magic Tap and the
        // escape gesture stop the recording into review (S6).
        .accessibilityAction(.magicTap, onMagicTap)
        .accessibilityAction(.escape, onStop)
    }

    @ViewBuilder
    private var middle: some View {
        if warning {
            Text("30 seconds left")
                .font(.caption.weight(.medium))
                .foregroundStyle(.orange)
                .lineLimit(1)
        } else {
            // Narrow first, the waveform goes (S2.4).
            ViewThatFits(in: .horizontal) {
                VoiceLiveWaveform(peaks: peaks, litBars: litBars)
                    .frame(minWidth: 40)
                Color.clear.frame(width: 0, height: 0)
            }
            .padding(.trailing, 4)
        }
    }
}

/// S2.9's haptics on an iPhone; an iPad has none.
enum VoiceHaptics {
    /// A recording's start: a light tap.
    static let tapIntensity = 0.6

    static func feedback(for cue: VoiceComposer.HapticCue?) -> SensoryFeedback? {
        guard let cue, UIDevice.current.userInterfaceIdiom == .phone else { return nil }
        switch cue.haptic {
        case .light: return .impact(weight: .light, intensity: tapIntensity)
        case .success: return .success
        case .warning: return .warning
        }
    }
}

#endif
