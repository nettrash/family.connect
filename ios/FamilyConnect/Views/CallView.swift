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
            // by itself.
            Color.clear.frame(height: 72)
        } else {
            HStack(spacing: 40) {
                roundButton(
                    calls.isMuted ? "mic.slash.fill" : "mic.fill",
                    tint: calls.isMuted ? .primary : .secondary,
                    label: calls.isMuted ? String(localized: "Unmute") : String(localized: "Mute")
                ) {
                    calls.toggleMute()
                }
                #if os(iOS)
                roundButton(
                    calls.isSpeaker ? "speaker.wave.3.fill" : "speaker.wave.1",
                    tint: calls.isSpeaker ? .primary : .secondary,
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

    private func roundButton(_ symbol: String, tint: Color, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.title2)
                .foregroundStyle(.white)
                .frame(width: 64, height: 64)
                .background(tint, in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}
