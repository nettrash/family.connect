//
//  CallVideoView.swift
//  FamilyConnect
//
//  The two video surfaces of a video call (docs/protocol.md, "Video"):
//  the far side full-bleed and the local preview picture-in-picture. One
//  SwiftUI representable over WebRTC's Metal renderer — RTCMTLVideoView
//  on iOS, RTCMTLNSVideoView on the Mac (the iOS class does not exist in
//  the macOS slice) — attached to the tracks through CallManager, which
//  holds the renderer until media exists and re-attaches it to a window
//  reopened mid-call.
//
//  Detach is the representable's job: dismantle hands the renderer back
//  as nil, so a dismissed call screen never leaves a dead Metal view
//  registered on a live track.
//

import SwiftUI
import WebRTC

/// Which of the call's two pictures this surface shows.
enum CallVideoRole {
    case local
    case remote
}

#if os(iOS)

struct CallVideoSurface: UIViewRepresentable {
    let role: CallVideoRole
    let calls: CallManager

    final class Coordinator {
        var detach: (() -> Void)?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> RTCMTLVideoView {
        let view = RTCMTLVideoView()
        // Fill the surface; a call window is never the frame's aspect.
        view.videoContentMode = .scaleAspectFill
        view.clipsToBounds = true
        attach(view, coordinator: context.coordinator)
        return view
    }

    func updateUIView(_ uiView: RTCMTLVideoView, context: Context) {}

    static func dismantleUIView(_ uiView: RTCMTLVideoView, coordinator: Coordinator) {
        coordinator.detach?()
    }

    private func attach(_ view: RTCMTLVideoView, coordinator: Coordinator) {
        let manager = calls
        let role = role
        switch role {
        case .local: manager.setLocalVideoRenderer(view)
        case .remote: manager.setRemoteVideoRenderer(view)
        }
        coordinator.detach = { [weak manager] in
            switch role {
            case .local: manager?.setLocalVideoRenderer(nil)
            case .remote: manager?.setRemoteVideoRenderer(nil)
            }
        }
    }
}

#elseif os(macOS)

struct CallVideoSurface: NSViewRepresentable {
    let role: CallVideoRole
    let calls: CallManager

    final class Coordinator {
        var detach: (() -> Void)?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> RTCMTLNSVideoView {
        let view = RTCMTLNSVideoView()
        // No videoContentMode on the Mac renderer; it letterboxes, which
        // in a resizable window is the right behavior anyway.
        view.wantsLayer = true
        attach(view, coordinator: context.coordinator)
        return view
    }

    func updateNSView(_ nsView: RTCMTLNSVideoView, context: Context) {}

    static func dismantleNSView(_ nsView: RTCMTLNSVideoView, coordinator: Coordinator) {
        coordinator.detach?()
    }

    private func attach(_ view: RTCMTLNSVideoView, coordinator: Coordinator) {
        let manager = calls
        let role = role
        switch role {
        case .local: manager.setLocalVideoRenderer(view)
        case .remote: manager.setRemoteVideoRenderer(view)
        }
        coordinator.detach = { [weak manager] in
            switch role {
            case .local: manager?.setLocalVideoRenderer(nil)
            case .remote: manager?.setRemoteVideoRenderer(nil)
            }
        }
    }
}

#endif
