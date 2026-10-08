//
//  RoundDoorWiringTests.swift
//  FamilyConnectTests
//
//  Whether an iPhone running THIS build offers a video message (#79, S1.2,
//  S1.4, S1.5): the composer's door needs the window's recorder presenter in
//  its environment (`videoRecorder != nil`), and the conversation is not the
//  root — it is pushed onto ChatListView's NavigationStack under RootView,
//  which applies `.videoMessageRecorderHost()`. A presenter that stopped at
//  the stack would hide the video button and the paperclip's Record Video
//  Message on every iPhone, with nothing else wrong. The rest of the door —
//  the server's key, the camera, the slot — is ComposerSlotTests' and the
//  record vectors'; the key's arrival is VideoDiscoveryTests'.
//
//  iOS only: the Mac's conversation is not pushed.
//

#if os(iOS)

import SwiftUI
import Testing
import UIKit
@testable import FamilyConnect

@MainActor
final class PresenterSeen {
    var presenter: VideoMessagePresenter?
    var appeared = false
}

private struct PresenterProbe: View {
    let seen: PresenterSeen
    @Environment(VideoMessagePresenter.self) private var presenter: VideoMessagePresenter?

    var body: some View {
        Color.clear
            .onAppear {
                seen.appeared = true
                seen.presenter = presenter
            }
    }
}

@MainActor
@Suite("Video message: the door's wiring on iPhone")
struct RoundDoorWiringTests {

    @Test("the root's recorder presenter reaches a conversation pushed onto the chat list's stack")
    func presenterReachesThePushedConversation() async throws {
        let seen = PresenterSeen()
        let api = APIClient(serverURL: URL(string: "https://door.invalid"))
        // RootView's shape: the host applied OUTSIDE the list, whose
        // NavigationStack pushes the conversation by chat id.
        let root = NavigationStack(path: .constant([Int64(42)])) {
            Color.clear
                .navigationDestination(for: Int64.self) { _ in PresenterProbe(seen: seen) }
        }
        .videoMessageRecorderHost()
        .environment(CallManager())
        .environment(AppSession(api: api, defaultServerURL: { nil }))

        let controller = UIHostingController(rootView: root)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = controller
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        for _ in 0..<20 where !seen.appeared {
            controller.view.layoutIfNeeded()
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        #expect(seen.appeared, "the pushed destination was never drawn")
        #expect(seen.presenter != nil,
                "a conversation pushed onto the stack has no recorder: no video button, no Record Video Message")
    }
}

#endif
