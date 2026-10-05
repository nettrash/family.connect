//
//  VoiceComposerRows.swift
//  FamilyConnect
//
//  What takes the field's place while a voice message is being made (#79,
//  docs/audio-video-messages-2026-10-04.md, S2.3, S2.4, S2.6, S2.9): the hold
//  row while a finger holds the microphone, the recording row once it is
//  hands-free, and the Undo row during the five seconds after a release.
//
//  IN THE ROW, NEVER ABOVE IT. The recording strip used to be stacked above
//  the input row, which grew the bar — and the bar's height must stay
//  constant, or the newest messages slide under it (ConversationView's
//  header). Each of these is drawn in the space the controls or the field
//  already occupy, at the same height, so nothing above them moves.
//
//  The red dot pulses once a second (steady under Reduce Motion); the timer
//  is m:ss in monospaced digits from the recorder's own clock; the level
//  meter is five bars lit at −50, −40, −30, −20 and −10 dBFS of the PEAK, the
//  measure every client shares (`AudioRecorder.litBars`). The clock is never
//  announced — only state changes are (S6).
//
//  The dot, the meter and the timer are shared: the Mac's recording row
//  (MacVoiceRecordingRow) draws the same three, so the two platforms cannot
//  drift on what "recording" looks like (S2.9). The rows themselves are the
//  phone's and the tablet's — a Mac has no hold, no Undo window and no
//  haptics.
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

/// While a finger holds the microphone: red dot, "0:00", the meter and
/// "‹ Slide to cancel" — red, and "Release to cancel", once cancel is armed.
struct VoiceHoldRow: View {
    let elapsed: TimeInterval
    let litBars: Int
    let armed: Bool
    let warning: Bool
    let height: CGFloat

    var body: some View {
        HStack(spacing: 8) {
            RecordingDot()
            VoiceTimerText(elapsed: elapsed, warning: warning)
            // Narrow first, the meter goes (S2.4).
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    meterOrWarning
                    Spacer(minLength: 4)
                    cancelCue
                }
                HStack(spacing: 8) {
                    Spacer(minLength: 4)
                    cancelCue
                }
            }
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(
            armed ? AnyShapeStyle(Color.red.opacity(0.16)) : AnyShapeStyle(.fill.tertiary),
            in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var meterOrWarning: some View {
        if warning {
            Text("30 seconds left")
                .font(.caption.weight(.medium))
                .foregroundStyle(.orange)
                .lineLimit(1)
        } else {
            VoiceLevelMeter(lit: litBars)
        }
    }

    @ViewBuilder
    private var cancelCue: some View {
        if armed {
            Text("Release to cancel")
                .font(.callout.weight(.semibold))
                .foregroundStyle(.red)
                .lineLimit(1)
        } else {
            HStack(spacing: 2) {
                Image(systemName: "chevron.backward")
                    .font(.caption.weight(.semibold))
                    .accessibilityHidden(true)
                Text("Slide to cancel")
                    .font(.callout)
                    .lineLimit(1)
            }
            .foregroundStyle(.secondary)
        }
    }
}

/// The lock above the slot while it is held: a lock over an up chevron,
/// eight points above the slot and outside the bar, so the bar keeps its
/// height (S2.3).
struct VoiceLockCue: View {
    static let height: CGFloat = 52

    var body: some View {
        VStack(spacing: 4) {
            Image(systemName: "lock.fill")
                .font(.footnote.weight(.semibold))
            Image(systemName: "chevron.up")
                .font(.caption2.weight(.bold))
        }
        .foregroundStyle(.secondary)
        .frame(width: 32, height: Self.height)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().strokeBorder(.quaternary))
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }
}

/// Hands-free: [trash] red dot, timer, meter [stop] — and the slot beside it
/// is the Send arrow. Beside words or staged items there is no Stop here:
/// the slot itself is Stop (S2.4).
struct VoiceRecordingRow: View {
    let elapsed: TimeInterval
    let litBars: Int
    let besideDraft: Bool
    let warning: Bool
    let stillRecording: Bool
    let control: CGFloat
    let onDelete: () -> Void
    let onStop: () -> Void
    let onMagicTap: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .font(.system(size: 18))
                    .foregroundStyle(.red)
                    .frame(width: control, height: control)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Delete recording")

            HStack(spacing: 8) {
                RecordingDot()
                VoiceTimerText(elapsed: elapsed, warning: warning)
                middle
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if !besideDraft {
                Button(action: onStop) {
                    Image(systemName: "stop.circle")
                        .font(.system(size: 22))
                        .foregroundStyle(.tint)
                        .frame(width: control, height: control)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Stop recording")
            }
        }
        .frame(maxWidth: .infinity, minHeight: control, maxHeight: control)
        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        // Wherever VoiceOver's focus is inside the row, Magic Tap and the
        // escape gesture stop the recording into review (S6).
        .accessibilityAction(.magicTap, onMagicTap)
        .accessibilityAction(.escape, onStop)
    }

    @ViewBuilder
    private var middle: some View {
        if stillRecording {
            Text("Still recording. Tap Send when you're done.")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
        } else if warning {
            Text("30 seconds left")
                .font(.caption.weight(.medium))
                .foregroundStyle(.orange)
                .lineLimit(1)
        } else {
            // Narrow first, the meter goes (S2.4).
            ViewThatFits(in: .horizontal) {
                VoiceLevelMeter(lit: litBars)
                Color.clear.frame(width: 0, height: 0)
            }
        }
    }
}

/// The five seconds after a release that sends: "[Undo]  Sending voice
/// message · 0:12" with a 2-point line along its bottom emptying over the
/// window — "Sending in 5", counting down once a second and not announced,
/// under Reduce Motion (S2.6). It takes the field's place only.
struct VoiceUndoRow: View {
    let recordedMS: UInt64
    let untilMS: UInt64
    let windowMS: UInt64
    let clock: () -> UInt64
    let onUndo: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 6) {
            Button("Undo", action: onUndo)
                .buttonStyle(.bordered)
                .controlSize(.small)
                // Never squeezed: it is the one way back.
                .fixedSize()
            // The length must survive a narrow field (S2.6 names it), so the
            // line shrinks rather than truncating.
            Text("Sending voice message · \(AudioRecorder.timeLabel(Double(recordedMS) / 1000))")
                .font(.footnote)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .allowsTightening(true)
                .layoutPriority(1)
            Spacer(minLength: 0)
            if reduceMotion {
                TimelineView(.periodic(from: .now, by: 1)) { _ in
                    Text("Sending in \(secondsLeft)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
            }
        }
        .padding(.horizontal, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(alignment: .bottom) {
            if !reduceMotion {
                TimelineView(.animation) { _ in
                    GeometryReader { geometry in
                        Rectangle()
                            .fill(.tint)
                            .frame(width: geometry.size.width * fractionLeft, height: 2)
                    }
                    .frame(height: 2)
                }
                .padding(.horizontal, 12)
                .accessibilityHidden(true)
            }
        }
    }

    private var remainingMS: UInt64 { RecordGesture.saturatingSub(untilMS, clock()) }

    private var fractionLeft: CGFloat {
        guard windowMS > 0 else { return 0 }
        return CGFloat(min(1, Double(remainingMS) / Double(windowMS)))
    }

    private var secondsLeft: Int {
        Int((Double(remainingMS) / 1000).rounded(.up))
    }
}

/// S2.9's haptics on an iPhone; an iPad has none.
///
/// The two impacts carry an INTENSITY as well as a weight, so the start at H
/// is felt as more than a tap's start whatever the weight does: on the iOS 27
/// SDK `SensoryFeedback.Weight.medium` and `.heavy` both hold `.light`
/// (`String(describing: SensoryFeedback.impact(weight: .medium))` prints
/// `Weight.Storage.light`, and `.impact(weight: .light) == .impact(weight:
/// .medium)`), which would make S2.9's light and medium the same buzz.
enum VoiceHaptics {
    /// A tap's start: lighter than the hold's.
    static let tapIntensity = 0.6
    /// The hold's start at H: full.
    static let holdIntensity = 1.0

    static func feedback(for cue: VoiceComposer.HapticCue?) -> SensoryFeedback? {
        guard let cue, UIDevice.current.userInterfaceIdiom == .phone else { return nil }
        switch cue.haptic {
        case .light: return .impact(weight: .light, intensity: tapIntensity)
        case .medium: return .impact(weight: .medium, intensity: holdIntensity)
        case .selection: return .selection
        case .success: return .success
        case .warning: return .warning
        }
    }
}

#endif
