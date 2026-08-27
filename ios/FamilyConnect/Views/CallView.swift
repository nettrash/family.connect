//
//  CallView.swift
//  FamilyConnect
//
//  The in-call screen, shared by the phone (a full-screen cover) and the
//  Mac (its own window): who, what is happening, and the three things
//  there are to do about it. Everything it shows is CallManager's state;
//  everything it does goes back through CallManager, which on the phone
//  routes answer and hang-up through CallKit so the system's call and the
//  app's stay one.
//

import SwiftUI

struct CallView: View {
    @Environment(CallManager.self) private var calls
    @Environment(\.colorScheme) private var colorScheme
    /// Ticks the timer while the call is active.
    @State private var elapsed = 0

    private var status: String {
        CallStatusText.line(phase: calls.phase, direction: calls.direction, elapsed: elapsed, video: calls.isVideo)
    }

    private var isIncomingRing: Bool {
        if case .incoming = calls.phase { return true }
        return false
    }

    private var isEnded: Bool {
        if case .ended = calls.phase { return true }
        return false
    }

    /// The far side's picture is up, so the face IS the who: the big
    /// identity block leaves the middle of the screen and a compact pill
    /// carries the name and the timer instead. The two are one flag so
    /// exactly one of them is ever on screen.
    private var showsCompactIdentity: Bool {
        calls.isVideo && calls.isRemoteVideoActive
    }

    #if os(iOS)
    /// The phone must not dim and lock in the middle of a VIDEO call:
    /// there is a face on the screen and nobody is touching it. A voice
    /// call is the opposite case — the phone is at an ear, and the idle
    /// timer (with the proximity sensor) is what stops a cheek from
    /// hanging up. So video only, and only while the call is actually
    /// going; the ring and the ended linger are short and keep the
    /// default.
    private var keepsScreenAwake: Bool {
        guard calls.isVideo else { return false }
        switch calls.phase {
        case .connecting, .active: return true
        default: return false
        }
    }
    #endif

    var body: some View {
        ZStack {
            // The far side, full bleed, under everything. Present for the
            // whole life of a video call — the renderer has to exist
            // before the track does, and the manager attaches it late.
            if calls.isVideo {
                CallVideoSurface(role: .remote, calls: calls)
                    .ignoresSafeArea()
            }
            VStack(spacing: 24) {
                Spacer()
                // Who and what: on screen for the whole of a voice call,
                // and on a video call until the far side's picture is up —
                // from then on the pill in the top-leading corner is the
                // who (and, more to the point, the timer).
                if !showsCompactIdentity {
                    InitialsAvatar(
                        title: calls.peerName,
                        userID: calls.peerUserID,
                        avatarVersion: calls.peerAvatarVersion,
                        size: 112)
                    VStack(spacing: 8) {
                        Text(calls.peerName)
                            .font(.title)
                            .fontWeight(.semibold)
                            .multilineTextAlignment(.center)
                        Text(status)
                            .font(.headline)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                            .contentTransition(.numericText())
                            .accessibilityLabel(status)
                        if calls.isVideo && calls.cameraDenied {
                            // A denied camera is a footnote, never an end
                            // reason (docs/protocol.md, "Video").
                            Text(CallStatusText.cameraDeniedNote)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .multilineTextAlignment(.center)
                        }
                    }
                    .padding(.horizontal, 32)
                }
                Spacer()
                // 16, not the identity block's 32: the control row is the
                // widest thing on the screen, and on the smallest iPhone
                // (375pt) a video call's five discs need every point —
                // 5 × 56 = 280 inside 375 − 2 × 16.
                controls
                    .padding(.horizontal, 16)
                    .padding(.bottom, 48)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // A video call is DARK, deterministically, the way FaceTime is.
        // The remote surface is a black Metal view until the first frame
        // decodes, and in the light appearance that put the identity
        // block's primary text — black — on it: black on black, on both
        // the phone and the Mac, for the whole of connecting. Forcing the
        // scheme (not just the ground) is what turns the text, the
        // material discs and the pill light in one go. A voice call has
        // no picture to sit on and keeps the app's appearance.
        .background(calls.isVideo ? Color.black : Color.appBackground)
        .environment(\.colorScheme, calls.isVideo ? .dark : colorScheme)
        .overlay(alignment: .topLeading) {
            // The name and the timer, while the identity block is off
            // screen — without this the timer vanished for the whole of a
            // video call the moment the far side's picture arrived.
            if showsCompactIdentity {
                identityPill
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, 20)
                    // The local preview's column is reserved on the right
                    // — its tile, its trailing padding and a gap — so the
                    // pill truncates before it can ever run under the
                    // tile, however long the name. Only WHILE the tile is
                    // there: with the camera off the tile is gone and the
                    // pill may have the width back — animated, so a long
                    // name does not snap between its two lengths.
                    .padding(.trailing, calls.isCameraOn ? Self.localPreviewSize.width + 20 + 12 : 20)
                    .padding(.top, 24)
                    .animation(.default, value: calls.isCameraOn)
            }
        }
        .overlay(alignment: .topTrailing) {
            // The local preview, picture-in-picture. Hidden — renderer
            // detached and all — while the camera is off: a black tile
            // saying nothing is worse than no tile.
            if calls.isVideo && calls.isCameraOn {
                CallVideoSurface(role: .local, calls: calls)
                    // A mirror while the FRONT camera captures — what a
                    // person expects of their own preview (FaceTime does
                    // it; Android's tile does it with setMirror). Only this
                    // tile flips: the far side keeps the true image, so a
                    // sign held up to the camera still reads there. The
                    // back camera is a window, not a mirror, and the Mac's
                    // one camera counts as front.
                    .scaleEffect(x: calls.isFrontCamera ? -1 : 1, y: 1)
                    .frame(width: Self.localPreviewSize.width, height: Self.localPreviewSize.height)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.quaternary))
                    .padding(.top, 24)
                    .padding(.trailing, 20)
            }
        }
        #if os(macOS)
        // The window is the person: "Call" told a Mac with three windows
        // open nothing about which one this was.
        .navigationTitle(calls.peerName)
        #endif
        #if os(iOS)
        .onChange(of: keepsScreenAwake, initial: true) { _, awake in
            UIApplication.shared.isIdleTimerDisabled = awake
        }
        .onDisappear {
            // The cover is dismissed by the manager going idle, and that
            // is the one transition an onChange on a view being torn down
            // cannot be relied on to see.
            UIApplication.shared.isIdleTimerDisabled = false
        }
        #endif
        .task(id: calls.phase) {
            guard case .active(let since) = calls.phase else { return }
            while !Task.isCancelled {
                elapsed = max(0, Int(calls.now().timeIntervalSince(since)))
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// The compact identity: name over status, on a material capsule so
    /// it reads over whatever the far side's camera is pointed at.
    private var identityPill: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(calls.peerName)
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
            Text(status)
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .contentTransition(.numericText())
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(.ultraThinMaterial, in: Capsule())
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var controls: some View {
        if isIncomingRing {
            // The same disc and the same inset as the active row below, so
            // the centred identity block does not hop the moment Accept
            // swaps this row for that one (CallControlRowMetrics).
            HStack(spacing: 64) {
                CallControlButton(
                    symbol: "phone.down.fill",
                    label: String(localized: "Decline"),
                    emphasis: .hangUp,
                    side: discSide
                ) {
                    calls.declineIncoming()
                }
                .keyboardShortcut(.cancelAction)
                CallControlButton(
                    // The glyph says what answering opens: a camera on a
                    // video call. The label stays "Accept" — that is the
                    // verb, on both kinds.
                    symbol: calls.isVideo ? "video.fill" : "phone.fill",
                    label: String(localized: "Accept"),
                    emphasis: .accept,
                    side: discSide
                ) {
                    calls.acceptIncoming()
                }
                .keyboardShortcut(.defaultAction)
            }
            .padding(CallControlRowMetrics.inset(video: calls.isVideo))
        } else if isEnded {
            // Nothing to do: the reason is on screen and the view goes away
            // by itself. Same height as the row it replaces, so the
            // centered column does not jump the instant a call ends.
            Color.clear.frame(height: controlRowHeight)
        } else {
            // Every button gets an equal share of the row and the row is
            // capped BY HOW MANY THERE ARE, so five discs fit the narrowest
            // phone without touching and three discs on a voice call (or
            // two on the Mac) sit together in the middle rather than at
            // the far ends of a phone-sized row.
            HStack(spacing: 0) {
                CallControlButton(
                    symbol: calls.isMuted ? "mic.slash.fill" : "mic.fill",
                    label: calls.isMuted ? String(localized: "Unmute") : String(localized: "Mute"),
                    isActive: calls.isMuted,
                    side: discSide
                ) {
                    calls.toggleMute()
                }
                .frame(maxWidth: .infinity)
                #if os(iOS)
                CallControlButton(
                    // Both filled: the earlier outline `speaker.wave.1`
                    // read as a lighter weight than every other glyph in
                    // the row.
                    symbol: calls.isSpeaker ? "speaker.wave.3.fill" : "speaker.wave.1.fill",
                    label: String(localized: "Speaker"),
                    isActive: calls.isSpeaker,
                    side: discSide
                ) {
                    calls.toggleSpeaker()
                }
                .frame(maxWidth: .infinity)
                #endif
                if calls.isVideo {
                    // The camera follows the mute pair's rule: ON is the
                    // neutral state, OFF the accented one.
                    CallControlButton(
                        symbol: calls.isCameraOn ? "video.fill" : "video.slash.fill",
                        label: calls.isCameraOn
                            ? String(localized: "Turn camera off")
                            : String(localized: "Turn camera on"),
                        isActive: !calls.isCameraOn,
                        side: discSide
                    ) {
                        calls.toggleCamera()
                    }
                    .frame(maxWidth: .infinity)
                    #if os(iOS)
                    // Front/back is a phone thing; the Mac has one camera.
                    if calls.isCameraOn {
                        CallControlButton(
                            symbol: "arrow.triangle.2.circlepath.camera",
                            label: String(localized: "Switch camera"),
                            side: discSide
                        ) {
                            calls.flipCamera()
                        }
                        .frame(maxWidth: .infinity)
                    }
                    #endif
                }
                CallControlButton(
                    symbol: "phone.down.fill",
                    label: String(localized: "Hang Up"),
                    emphasis: .hangUp,
                    side: discSide
                ) {
                    calls.hangUp()
                }
                .frame(maxWidth: .infinity)
            }
            .frame(maxWidth: CallControlRowMetrics.rowWidth(buttons: activeButtonCount))
            .padding(CallControlRowMetrics.inset(video: calls.isVideo))
            .background {
                // On a video call the row sits on the far side's picture,
                // and glyph-on-material discs alone do not read over a
                // bright face. The backdrop is on for the WHOLE video call
                // rather than only while the remote picture is up: the
                // picture flag can flip both ways mid-call (best effort on
                // the far side's track), and a row that restyles itself
                // each time is worse than a plate that was always there.
                // A voice call's row sits on the app background and needs
                // nothing.
                if calls.isVideo {
                    RoundedRectangle(cornerRadius: 28, style: .continuous)
                        .fill(.ultraThinMaterial)
                }
            }
        }
    }

    /// How many discs the active row is showing right now — the mirror of
    /// the `#if`s and `if`s in `controls`, which is what sizes the row.
    private var activeButtonCount: Int {
        var count = 2 // mute, hang up
        #if os(iOS)
        count += 1 // speaker
        #endif
        if calls.isVideo {
            count += 1 // camera
            #if os(iOS)
            if calls.isCameraOn { count += 1 } // flip
            #endif
        }
        return count
    }

    /// The local preview's tile: portrait on a phone, whose camera is
    /// portrait, and landscape on a Mac, whose camera is landscape —
    /// letterboxing a 16:9 picture into a 110×150 tile left a strip of
    /// face between two black bars.
    private static let localPreviewSize: CGSize = {
        #if os(macOS)
        CGSize(width: 176, height: 99)
        #else
        CGSize(width: 110, height: 150)
        #endif
    }()

    /// A video call's five discs are 56pt so they fit the narrowest phone
    /// (see `controls`); a voice call's three keep the classic 64.
    private var discSide: CGFloat {
        CallControlRowMetrics.discSide(video: calls.isVideo)
    }

    /// The control row's height — disc plus, on a video call, the
    /// backdrop's inset on both sides — which is what the ended-state
    /// placeholder has to match.
    private var controlRowHeight: CGFloat {
        CallControlRowMetrics.rowHeight(video: calls.isVideo)
    }
}

/// The control row's geometry, in one place because THREE rows have to
/// agree on it: the incoming ring's Decline/Accept pair, the active
/// call's row, and the ended-state placeholder that stands in for the
/// row for the two seconds the reason shows. Each replaces the last in
/// the same centred column, and any height they do not share is a hop of
/// the identity block — on Accept, and on a missed call. Pure, so a test
/// can read the numbers without a CallManager.
nonisolated enum CallControlRowMetrics {
    /// A voice call's discs (and, historically, every disc).
    static let voiceDiscSide: CGFloat = 64
    /// A video call's discs: 56, so five fit the narrowest phone.
    static let videoDiscSide: CGFloat = 56
    /// The material plate's inset around a video call's row.
    static let backdropInset: CGFloat = 12
    /// The active row's cap: five 56pt discs breathe in it.
    static let maxWidth: CGFloat = 420
    /// The share of the row one disc gets before the cap: on a 780pt Mac
    /// window two voice-call discs then sit 100pt apart, not 210.
    static let slotWidth: CGFloat = 100

    static func discSide(video: Bool) -> CGFloat {
        video ? videoDiscSide : voiceDiscSide
    }

    /// The padding every row wears: the plate's inset on a video call
    /// (drawn or not — the ring row has no plate but keeps the footprint),
    /// nothing on a voice call.
    static func inset(video: Bool) -> CGFloat {
        video ? backdropInset : 0
    }

    /// The one height all three rows come to.
    static func rowHeight(video: Bool) -> CGFloat {
        discSide(video: video) + inset(video: video) * 2
    }

    /// The active row's width for a given number of discs, capped.
    static func rowWidth(buttons: Int) -> CGFloat {
        min(maxWidth, CGFloat(buttons) * slotWidth)
    }
}

/// One disc in the call screen's control row.
///
/// COLOR RULE: no state is ever a plain white disc. The previous pair
/// (active = white disc with a black glyph, the system Phone app's look)
/// was reported as "totally white in dark theme" — a white disc on a
/// dark call screen is a flashlight, and on a video call it sits on top
/// of a face. So a toggle is a material disc with the primary glyph while
/// inactive (it takes the appearance's colour, light or dark, and reads
/// over video), and the ACCENT disc with a white glyph while active
/// (muted, speaker on, camera off) — the app's own tint says "this is
/// on", in both appearances, on both platforms. Hang-up and decline are
/// red, accept is green, both with white glyphs, as every call UI does
/// it; flip-camera is a plain action and wears the inactive look.
struct CallControlButton: View {
    enum Emphasis {
        /// A toggle (mute, speaker, camera) or a plain action (flip).
        case toggle
        /// Hang up or decline.
        case hangUp
        /// Answer.
        case accept
    }

    let symbol: String
    /// The accessibility label — every disc has one, since the glyph is
    /// the only thing on it. Doubles as the Mac's tooltip.
    let label: String
    var isActive: Bool = false
    var emphasis: Emphasis = .toggle
    var side: CGFloat = 64
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                // Sized from the disc, not the text ramp: .title2 is 22pt
                // on a phone and 17 on a Mac, so the same disc carried a
                // visibly smaller glyph there. 0.34 of 64 is the phone's
                // 22.
                .font(.system(size: side * 0.34, weight: .medium))
                .foregroundStyle(glyphColor)
                .frame(width: side, height: side)
                .background { disc }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .help(label)
    }

    @ViewBuilder
    private var disc: some View {
        switch emphasis {
        case .toggle where isActive:
            Circle().fill(Color.accentColor)
        case .toggle:
            // A material on a flat ground is that ground: on a Mac voice
            // call (windowBackgroundColor) the inactive disc was invisible
            // and the glyph floated, and a light-mode phone was barely
            // better. The hairline is the balloon's own idiom
            // (MacMessageRow) — enough edge to be a button, too faint to
            // be a white disc.
            Circle()
                .fill(.ultraThinMaterial)
                .overlay(Circle().strokeBorder(Color.primary.opacity(0.15)))
        case .hangUp:
            Circle().fill(.red)
        case .accept:
            Circle().fill(.green)
        }
    }

    private var glyphColor: Color {
        if case .toggle = emphasis, !isActive { return .primary }
        return .white
    }
}

#Preview("Controls") {
    HStack(spacing: 16) {
        CallControlButton(symbol: "mic.fill", label: "Mute") {}
        CallControlButton(symbol: "mic.slash.fill", label: "Unmute", isActive: true) {}
        CallControlButton(symbol: "phone.down.fill", label: "Hang Up", emphasis: .hangUp) {}
        CallControlButton(symbol: "phone.fill", label: "Accept", emphasis: .accept) {}
    }
    .padding()
}
