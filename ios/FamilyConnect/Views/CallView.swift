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
                // from then on the face IS the who.
                if !(calls.isVideo && calls.isRemoteVideoActive) {
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
                }
                Spacer()
                controls
                    .padding(.bottom, 48)
            }
            .padding(.horizontal, 32)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appBackground)
        .overlay(alignment: .topTrailing) {
            // The local preview, picture-in-picture. Hidden — renderer
            // detached and all — while the camera is off: a black tile
            // saying nothing is worse than no tile.
            if calls.isVideo && calls.isCameraOn {
                CallVideoSurface(role: .local, calls: calls)
                    .frame(width: 110, height: 150)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.quaternary))
                    .padding(.top, 24)
                    .padding(.trailing, 20)
            }
        }
        .task(id: calls.phase) {
            guard case .active(let since) = calls.phase else { return }
            while !Task.isCancelled {
                elapsed = max(0, Int(calls.now().timeIntervalSince(since)))
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    @ViewBuilder
    private var controls: some View {
        if isIncomingRing {
            HStack(spacing: 64) {
                roundButton("phone.down.fill", tint: .red, label: String(localized: "Decline")) {
                    calls.declineIncoming()
                }
                roundButton("phone.fill", tint: .green, label: String(localized: "Accept")) {
                    calls.acceptIncoming()
                }
            }
        } else if isEnded {
            // Nothing to do: the reason is on screen and the view goes away
            // by itself. Same height as a button row, so the centered
            // column does not jump the instant a call ends.
            Color.clear.frame(height: Self.buttonSide)
        } else {
            // Five buttons on a video call (phone: mute, speaker, camera,
            // flip, hang up) need tighter spacing than voice's three.
            HStack(spacing: calls.isVideo ? 24 : 40) {
                // Fixed colors on the toggles, never .primary/.secondary:
                // .primary is WHITE in dark mode, which painted the active
                // state’s white glyph on a white disc. The system Phone
                // app’s pair instead — active is a white disc with a black
                // glyph, inactive a gray disc with a white one — reads the
                // same in both appearances, on both platforms.
                roundButton(
                    calls.isMuted ? "mic.slash.fill" : "mic.fill",
                    tint: calls.isMuted ? .white : .gray,
                    glyph: calls.isMuted ? .black : .white,
                    label: calls.isMuted ? String(localized: "Unmute") : String(localized: "Mute")
                ) {
                    calls.toggleMute()
                }
                #if os(iOS)
                roundButton(
                    calls.isSpeaker ? "speaker.wave.3.fill" : "speaker.wave.1",
                    tint: calls.isSpeaker ? .white : .gray,
                    glyph: calls.isSpeaker ? .black : .white,
                    label: String(localized: "Speaker")
                ) {
                    calls.toggleSpeaker()
                }
                #endif
                if calls.isVideo {
                    // The camera pair follows the mute pair's color rule:
                    // ON is the gray disc (the neutral state), OFF the
                    // white disc with the black glyph.
                    roundButton(
                        calls.isCameraOn ? "video.fill" : "video.slash.fill",
                        tint: calls.isCameraOn ? .gray : .white,
                        glyph: calls.isCameraOn ? .white : .black,
                        label: calls.isCameraOn
                            ? String(localized: "Turn camera off")
                            : String(localized: "Turn camera on")
                    ) {
                        calls.toggleCamera()
                    }
                    #if os(iOS)
                    // Front/back is a phone thing; the Mac has one camera.
                    if calls.isCameraOn {
                        roundButton(
                            "arrow.triangle.2.circlepath.camera",
                            tint: .gray,
                            label: String(localized: "Switch camera")
                        ) {
                            calls.flipCamera()
                        }
                    }
                    #endif
                }
                roundButton("phone.down.fill", tint: .red, label: String(localized: "Hang Up")) {
                    calls.hangUp()
                }
            }
        }
    }

    /// One diameter for the buttons AND the ended-state placeholder, so
    /// the two heights cannot drift apart again.
    private static let buttonSide: CGFloat = 64

    private func roundButton(_ symbol: String, tint: Color, glyph: Color = .white, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.title2)
                .foregroundStyle(glyph)
                .frame(width: Self.buttonSide, height: Self.buttonSide)
                .background(tint, in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}
