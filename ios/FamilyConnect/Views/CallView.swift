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
        CallStatusText.line(phase: calls.phase, direction: calls.direction, elapsed: elapsed)
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
        VStack(spacing: 24) {
            Spacer()
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
            }
            Spacer()
            controls
                .padding(.bottom, 48)
        }
        .padding(.horizontal, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appBackground)
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
            HStack(spacing: 40) {
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
