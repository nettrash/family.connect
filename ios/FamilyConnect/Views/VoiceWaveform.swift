//
//  VoiceWaveform.swift
//  FamilyConnect
//
//  The pieces every voice surface draws its shape with (#79, the approved
//  design "Voice and Video Messages"):
//
//    - `VoiceWaveformBars`: a voice note's 48 levels as however many bars fit
//      the width (`Waveform.bars`), each `Waveform.barFraction` of the
//      height, the first `Waveform.playedBars` of them in the played colour.
//      The bubble, the review chip and the "not sent" row all draw it.
//    - `VoiceLiveWaveform`: the recording row's level history scrolling in
//      from the trailing edge — the recorder's own peaks, one bar a tick.
//      Under Reduce Motion it is the steady five-bar meter instead: nothing
//      sweeps.
//    - `VoicePlaybackSpeed`: 1× → 1.5× → 2×, remembered per device and
//      applied through the player's rate.
//
//  Platform-free — the same ink on iPhone, iPad and the Mac, in the app's
//  tint and the system's own fills, so dark mode and Increase Contrast come
//  from the system rather than from fixed colours.
//

import Observation
import SwiftUI

// MARK: - The bars

struct VoiceWaveformBars: View {
    /// The note's 48 levels (`Waveform.levelsOrPlaceholder`).
    let levels: [UInt8]
    /// How many of them are played — counted for the bars actually drawn
    /// by `playedFraction`, so a bar lights at the same moment everywhere.
    var playedFraction: (positionMS: UInt64, durationMS: UInt64)? = nil
    var played: Color = .accentColor
    var unplayed: Color = .secondary
    var barWidth: CGFloat = 3
    var spacing: CGFloat = 2

    /// How many bars fit `width` — never fewer than one, never more than
    /// the 48 the wire carries (a bar is a level; more would invent shape).
    static func barCount(width: CGFloat, barWidth: CGFloat = 3, spacing: CGFloat = 2) -> Int {
        guard width > 0 else { return 1 }
        let count = Int(((width + spacing) / (barWidth + spacing)).rounded(.down))
        return min(max(1, count), Waveform.levelCount)
    }

    var body: some View {
        Canvas { context, size in
            let count = Self.barCount(width: size.width, barWidth: barWidth, spacing: spacing)
            let bars = Waveform.bars(levels, count: count)
            let lit = playedFraction.map {
                Waveform.playedBars(positionMS: $0.positionMS, durationMS: $0.durationMS, bars: count)
            } ?? 0
            // Bars keep their own width and gap, the row starting at the
            // leading edge: a wide row shows the same 48 bars, never fatter
            // ones. A narrow one shares what there is evenly.
            let step = min(barWidth + spacing, (size.width + spacing) / CGFloat(count))
            let width = max(1, step - spacing)
            for (index, level) in bars.enumerated() {
                let height = max(2, size.height * Waveform.barFraction(level))
                let rect = CGRect(
                    x: CGFloat(index) * step,
                    y: (size.height - height) / 2,
                    width: width,
                    height: height)
                context.fill(
                    Path(roundedRect: rect, cornerRadius: min(width, height) / 2),
                    with: .color(index < lit ? played : unplayed))
            }
        }
        .accessibilityHidden(true)
    }
}

// MARK: - The live waveform while recording

/// The recording's peaks so far, the newest at the trailing edge, in the
/// recording red. A peak becomes a level by the shared rule
/// (`Waveform.level`), so the bars the person watched are the bars the note
/// is sent with.
struct VoiceLiveWaveform: View {
    let peaks: [Float]
    /// Five bars lit by the latest peak — what Reduce Motion shows instead.
    let litBars: Int
    var height: CGFloat = 24

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static let barWidth: CGFloat = 3
    private static let spacing: CGFloat = 2

    /// The newest `count` levels, padded with silence at the leading edge
    /// before there are that many — so it scrolls in from the trailing edge.
    static func visibleLevels(peaks: [Float], count: Int) -> [UInt8] {
        guard count > 0 else { return [] }
        let recent = peaks.suffix(count).map { Waveform.level(Double($0)) }
        return [UInt8](repeating: 0, count: count - recent.count) + recent
    }

    var body: some View {
        if reduceMotion {
            VoiceLevelMeter(lit: litBars)
        } else {
            Canvas { context, size in
                let count = max(1, Int(((size.width + Self.spacing) / (Self.barWidth + Self.spacing)).rounded(.down)))
                let levels = Self.visibleLevels(peaks: peaks, count: count)
                let step = Self.barWidth + Self.spacing
                // Flush with the trailing edge, where the newest bar is.
                let origin = size.width - CGFloat(count) * step + Self.spacing
                for (index, level) in levels.enumerated() {
                    let barHeight = max(2, size.height * Waveform.barFraction(level))
                    let rect = CGRect(
                        x: origin + CGFloat(index) * step,
                        y: (size.height - barHeight) / 2,
                        width: Self.barWidth,
                        height: barHeight)
                    context.fill(
                        Path(roundedRect: rect, cornerRadius: Self.barWidth / 2),
                        with: .color(Color.red.opacity(0.8)))
                }
            }
            .frame(height: height)
            .frame(minWidth: 24, maxWidth: .infinity)
            .accessibilityHidden(true)
        }
    }
}

// MARK: - Playback speed

/// 1× → 1.5× → 2× for voice messages, remembered on this device and shared
/// by every bubble: changing it on one changes the rest, and the one that
/// plays takes the new rate at once.
@MainActor
@Observable
final class VoicePlaybackSpeed {
    static let shared = VoicePlaybackSpeed()

    nonisolated static let rates: [Double] = [1, 1.5, 2]

    private(set) var rate: Double

    @ObservationIgnored private let read: () -> Double
    @ObservationIgnored private let write: (Double) -> Void

    init(
        read: @escaping () -> Double = { AppSettings.voicePlaybackRate },
        write: @escaping (Double) -> Void = { AppSettings.voicePlaybackRate = $0 }
    ) {
        self.read = read
        self.write = write
        rate = read()
    }

    /// The next speed round the cycle.
    nonisolated static func next(after rate: Double) -> Double {
        guard let index = rates.firstIndex(of: rate) else { return rates[0] }
        return rates[(index + 1) % rates.count]
    }

    func cycle() {
        set(Self.next(after: rate))
    }

    /// A speed chosen outright — the Mac menu's submenu.
    func set(_ newRate: Double) {
        guard Self.rates.contains(newRate) else { return }
        rate = newRate
        write(newRate)
    }

    /// "1×", "1.5×", "2×" — the decimal written the way the language writes
    /// it (the catalogue's translations).
    nonisolated static func label(_ rate: Double) -> String {
        switch rate {
        case 1.5: String(localized: "1.5×")
        case 2: String(localized: "2×")
        default: String(localized: "1×")
        }
    }

    var label: String { Self.label(rate) }
}

/// The chip: the current speed in the tint, on a soft wash of it; a tap
/// moves round the cycle. Shown while a note plays.
struct VoiceSpeedChip: View {
    let speed: VoicePlaybackSpeed
    /// Ink and wash: the tint on a received bubble; white on the tint-filled
    /// own bubble.
    let ink: Color

    var body: some View {
        Button {
            speed.cycle()
        } label: {
            Text(verbatim: speed.label)
                .font(.caption2.weight(.bold).monospacedDigit())
                .foregroundStyle(ink)
                .padding(.horizontal, 7)
                .padding(.vertical, 1)
                .background(ink.opacity(0.16), in: Capsule())
                // The chip stays small; its target grows toward 44 (S1.1)
                // — sideways and downward only: right above it are the
                // waveform's trailing bars, whose tap seeks.
                .contentShape(VoiceSpeedChipTarget())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(String(localized: "Playback speed, \(speed.label)")))
        .help(Text("Playback speed"))
    }
}

/// The speed chip's tap target: the chip and `slack` points to each side and
/// below it, nothing above — the waveform it sits under keeps every point
/// of its own.
struct VoiceSpeedChipTarget: Shape {
    static let slack: CGFloat = 10

    func path(in rect: CGRect) -> Path {
        Path(CGRect(
            x: rect.minX - Self.slack, y: rect.minY,
            width: rect.width + 2 * Self.slack, height: rect.height + Self.slack))
    }
}
