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
//  BY NAME, so a dismissed call screen never leaves a dead Metal view
//  registered on a live track — and, just as important, never unhooks a
//  live one. Two view trees can overlap (a call screen rebuilt while the
//  call runs), and a blind "clear whatever is registered" from the dying
//  surface would leave the call with no picture for the rest of its life
//  and nothing to re-attach it (issue #38). CallManager ignores a
//  dismantle that does not name the current renderer, and logs that it
//  did.
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
        /// Given the very view being dismantled, so the detach names it
        /// rather than clearing whatever happens to be registered.
        var detach: ((any RTCVideoRenderer) -> Void)?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> RTCMTLVideoView {
        let view = RTCMTLVideoView()
        // Fill the surface; a call window is never the frame's aspect —
        // on a PHONE, where the far end is another phone held the same
        // way and the crop is a sliver. On an iPad the far end's portrait
        // frame filling a landscape 13-inch screen showed ~40% of it, the
        // face usually outside; the remote picture is fitted there and
        // the surface's black shows either side, as FaceTime does. The
        // local preview tile keeps filling: it is a thumbnail.
        let fitsRemote = role == .remote && UIDevice.current.userInterfaceIdiom == .pad
        view.videoContentMode = fitsRemote ? .scaleAspectFit : .scaleAspectFill
        view.clipsToBounds = true
        attach(view, coordinator: context.coordinator)
        return view
    }

    func updateUIView(_ uiView: RTCMTLVideoView, context: Context) {}

    static func dismantleUIView(_ uiView: RTCMTLVideoView, coordinator: Coordinator) {
        coordinator.detach?(uiView)
    }

    private func attach(_ view: RTCMTLVideoView, coordinator: Coordinator) {
        let manager = calls
        let role = role
        switch role {
        case .local: manager.setLocalVideoRenderer(view)
        case .remote: manager.setRemoteVideoRenderer(view)
        }
        coordinator.detach = { [weak manager] dismantled in
            switch role {
            case .local: manager?.detachLocalVideoRenderer(dismantled)
            case .remote: manager?.detachRemoteVideoRenderer(dismantled)
            }
        }
    }
}

#elseif os(macOS)

struct CallVideoSurface: NSViewRepresentable {
    let role: CallVideoRole
    let calls: CallManager

    final class Coordinator {
        /// Given the very view being dismantled, so the detach names it
        /// rather than clearing whatever happens to be registered.
        var detach: ((any RTCVideoRenderer) -> Void)?
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
        coordinator.detach?(nsView)
    }

    private func attach(_ view: RTCMTLNSVideoView, coordinator: Coordinator) {
        let manager = calls
        let role = role
        switch role {
        case .local: manager.setLocalVideoRenderer(view)
        case .remote: manager.setRemoteVideoRenderer(view)
        }
        coordinator.detach = { [weak manager] dismantled in
            switch role {
            case .local: manager?.detachLocalVideoRenderer(dismantled)
            case .remote: manager?.detachRemoteVideoRenderer(dismantled)
            }
        }
    }
}

#endif
