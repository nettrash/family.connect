//
//  VideoMessageRecorderView.swift
//  FamilyConnect
//
//  The round-video recorder on screen (#79, Phase 3 —
//  docs/audio-video-messages-2026-10-04.md, S3.3, S3.4, S6, S8.1–S8.3).
//
//  A LAYER OF THE WINDOW'S ROOT, never a sheet, a dialog or a popover. On
//  iPhone and iPad it sits in `RootView` beneath the call's
//  `.fullScreenCover` — a call's cover simply rises over it, and it is still
//  there in REVIEW when the call ends (S4) — and on the Mac it lies over the
//  window's root, the sidebar included. The toolbar a Mac window draws cannot
//  be covered by anything the content draws, so what is beneath is disabled
//  instead (`VideoMessageRecorderHost`, `macRecorderOpen`).
//
//  It draws `VideoMessageSession` and does nothing of its own: every control
//  is one call on the session, which asks `VideoRecorderMachine`.
//

import AVFoundation
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// MARK: - The host

extension View {
    /// The window's recorder: a presenter for the composers below, and the
    /// recorder drawn over all of it while one is open.
    func videoMessageRecorderHost() -> some View {
        modifier(VideoMessageRecorderHost())
    }
}

struct VideoMessageRecorderHost: ViewModifier {
    @State private var presenter = VideoMessagePresenter()
    @Environment(CallManager.self) private var calls
    @Environment(AppSession.self) private var session
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    #if os(macOS)
    @State private var windowBox = MacWindowBox()
    @State private var closeGuard = MacWindowCloseGuard()
    @State private var keyMonitor: Any?
    #endif

    func body(content: Content) -> some View {
        content
            // The conversation cannot change underneath it, and neither Tab
            // nor a screen reader leaves the recorder (S3.3, S3.4).
            .disabled(presenter.isOpen)
            .accessibilityHidden(presenter.isOpen)
            .overlay {
                if let recorder = presenter.session {
                    VideoMessageRecorderView(session: recorder, paneFrame: presenter.paneFrame)
                        .transition(.opacity)
                }
            }
            // A 200 ms fade, none under Reduce Motion (S1.1).
            .animation(
                reduceMotion ? nil : .easeInOut(duration: Double(RecordRules.recorderFadeMS) / 1000),
                value: presenter.isOpen)
            .environment(presenter)
            // A call rang, started or was placed: PREVIEW closes, a take
            // stops into REVIEW, REVIEW stays (S4).
            .onChange(of: calls.isIdle) { _, idle in
                if !idle { presenter.session?.callStarted() }
            }
            // Signed out: everything recorded and not sent is deleted (S4).
            .onChange(of: session.phase) { _, phase in
                if phase != .active { presenter.session?.signedOut() }
            }
            #if os(macOS)
            .background(MacHostWindowReader(box: windowBox))
            .onAppear { wireMac() }
            .onChange(of: presenter.isOpen) { _, open in
                if open { openedOnMac() } else { closedOnMac() }
            }
            // The window lost focus but is still on screen: PREVIEW closes —
            // never to a permission prompt the recorder raised (S3.4, S4).
            .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResignKeyNotification)) { note in
                guard let window = note.object as? NSWindow, window === windowBox.window else { return }
                presenter.session?.focusLost()
            }
            // The menu bar's commands and Refresh act on the key window; none
            // of them while its recorder is open (S8.3).
            .focusedSceneValue(\.macRecorderOpen, presenter.isOpen)
            #endif
    }

    #if os(macOS)
    private func wireMac() {
        let box = windowBox
        presenter.window = { box.window }
        presenter.proceed = { closing in
            switch closing {
            case .window: box.window?.close()
            case .quit: NSApp.terminate(nil)
            }
        }
    }

    /// ⌘W and the close button ask over a clip (S8.3), and the recorder's
    /// keys are caught before any control's (S3.4).
    private func openedOnMac() {
        guard let window = windowBox.window else { return }
        let presenter = presenter
        closeGuard.shouldClose = { presenter.session?.windowShouldClose() ?? true }
        closeGuard.install(on: window)
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard event.window === window, let session = presenter.session,
                  session.state.question == nil
            else { return event }
            return VideoRecorderKeys.handle(
                keyCode: event.keyCode,
                modifiers: event.modifierFlags.intersection(.deviceIndependentFlagsMask),
                session: session) ? nil : event
        }
    }

    private func closedOnMac() {
        closeGuard.uninstall()
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = nil
    }
    #endif
}

/// Reports where a conversation lies in the window, for the recorder to lay
/// itself out over (S3.3: the circle and the controls over the conversation
/// pane; the scrim over everything).
struct VideoPaneReporter: ViewModifier {
    @Binding var frame: CGRect?
    let presenter: VideoMessagePresenter?

    func body(content: Content) -> some View {
        content.onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .global)
        } action: { new in
            frame = new
            if let presenter, presenter.isOpen { presenter.paneFrame = new }
        }
    }
}

/// The recorder's keys on a desktop keyboard (S3.4, S6): Return is the
/// slot, Esc closes or stops or asks, and in REVIEW Space plays and pauses
/// WHEREVER focus is — it never sends, deletes or retakes.
@MainActor
enum VideoRecorderKeys {
    static let returnKey: UInt16 = 36
    static let enterKey: UInt16 = 76
    static let escapeKey: UInt16 = 53
    static let spaceKey: UInt16 = 49

    #if os(macOS)
    /// True when the key was the recorder's.
    static func handle(keyCode: UInt16, modifiers: NSEvent.ModifierFlags, session: VideoMessageSession) -> Bool {
        guard modifiers.subtracting([.capsLock, .numericPad, .function]).isEmpty else { return false }
        switch keyCode {
        case escapeKey:
            session.escape()
            return true
        case returnKey, enterKey:
            session.activateSlot()
            return true
        case spaceKey:
            guard case .review = session.state.phase else { return false }
            session.playPause()
            return true
        default:
            return false
        }
    }
    #endif
}

private struct MacRecorderOpenKey: FocusedValueKey {
    typealias Value = Bool
}

extension FocusedValues {
    /// The key window's recorder is open (S8.3): the menu bar's commands that
    /// act on its conversation, and Refresh, are disabled.
    var macRecorderOpen: Bool? {
        get { self[MacRecorderOpenKey.self] }
        set { self[MacRecorderOpenKey.self] = newValue }
    }
}

// MARK: - The recorder

struct VideoMessageRecorderView: View {
    let session: VideoMessageSession
    let paneFrame: CGRect?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
    #endif
    @Environment(\.openURL) private var openURL
    @AccessibilityFocusState private var slotReadFocus: Bool
    @FocusState private var slotKeyFocus: Bool
    /// The layout chosen when RECORDING started, kept until Stop (S3.3).
    @State private var lockedSideways: Bool?

    private typealias Machine = VideoRecorderMachine

    private var state: Machine.State { session.state }

    var body: some View {
        GeometryReader { outer in
            let pane = localPane(in: outer)
            ZStack(alignment: .topLeading) {
                scrim(size: outer.size, pane: pane)
                recorder(in: pane)
                    .frame(width: pane.width, height: pane.height)
                    .offset(x: pane.minX, y: pane.minY)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .accessibilityLabel(Text("Video message"))
        // Magic Tap: Record, Stop, play/pause (S6); the escape gesture is
        // Esc (S3.4).
        #if os(iOS)
        .accessibilityAction(.magicTap) { session.magicTap() }
        #endif
        .accessibilityAction(.escape) { session.escape() }
        .alert("Delete video message?", isPresented: questionShown) {
            Button("Delete", role: .destructive) { session.answer(delete: true) }
            Button("Keep", role: .cancel) { session.answer(delete: false) }
        }
        .onAppear { focusSlot() }
        .onChange(of: phaseKey) { _, _ in
            // Focus stays on the slot as it changes (S3.4).
            focusSlot()
            if case .recording = state.phase {
                if lockedSideways == nil { lockedSideways = sideways }
            } else if case .finishing = state.phase {
            } else {
                lockedSideways = nil
            }
        }
        #if os(iOS)
        .background { keyboardDoors }
        #endif
    }

    // MARK: Where it lies

    /// The pane in this view's own coordinates: the conversation the
    /// composer reported, else the whole window.
    private func localPane(in proxy: GeometryProxy) -> CGRect {
        let bounds = CGRect(origin: .zero, size: proxy.size)
        guard let paneFrame else { return bounds }
        let origin = proxy.frame(in: .global).origin
        let local = paneFrame.offsetBy(dx: -origin.x, dy: -origin.y).intersection(bounds)
        return local.isNull || local.width < 200 || local.height < 200 ? bounds : local
    }

    private var isCompactWidth: Bool {
        #if os(iOS)
        sizeClass == .compact
        #else
        false
        #endif
    }

    /// Black at 70 % — over the conversation only 30 % on a regular width,
    /// so the message being answered stays readable — and opaque under Reduce
    /// Transparency. It takes all input (S3.3).
    /// The window darkens almost to black behind the recorder (the approved
    /// design), so the circle is the one bright thing on screen.
    static let scrimOpacity: Double = 0.86

    private func scrim(size: CGSize, pane: CGRect) -> some View {
        let whole = CGRect(origin: .zero, size: size)
        let dimPane = !isCompactWidth && pane != whole && !reduceTransparency
        return ZStack(alignment: .topLeading) {
            if dimPane {
                Path { path in
                    path.addRect(whole)
                    path.addRect(pane)
                }
                .fill(Color.black.opacity(0.7), style: FillStyle(eoFill: true))
                Color.black.opacity(0.3)
                    .frame(width: pane.width, height: pane.height)
                    .offset(x: pane.minX, y: pane.minY)
            } else {
                Color.black.opacity(reduceTransparency ? 1 : Self.scrimOpacity)
            }
        }
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        .ignoresSafeArea()
        .contentShape(Rectangle())
        .onTapGesture {}
        .accessibilityHidden(true)
    }

    // MARK: Layout (S3.3)

    @State private var paneSize: CGSize = .zero

    /// A pane shorter than 480 — a phone on its side — stands the controls
    /// in a column at the trailing edge.
    private var sideways: Bool { paneSize.height < 480 && paneSize.height > 0 }

    private var bannerHeight: CGFloat { session.reply == nil ? 0 : 52 }

    private func diameter(for size: CGSize, sideways: Bool) -> CGFloat {
        let raw = sideways
            ? min(320, size.height - 96, size.width - 200)
            : min(320, size.width - 48, size.height - 240 - bannerHeight)
        return max(160, raw)
    }

    @ViewBuilder
    private func recorder(in pane: CGRect) -> some View {
        let size = pane.size
        let isSideways = lockedSideways ?? (size.height < 480)
        let d = diameter(for: size, sideways: isSideways)
        Group {
            if isSideways {
                HStack(spacing: 16) {
                    VStack(spacing: 12) {
                        statusLine
                        circle(d)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    VStack(spacing: 12) {
                        replyBanner
                        controls(vertical: true)
                    }
                    .frame(width: 200)
                }
                .padding(.horizontal, 16)
            } else {
                VStack(spacing: 12) {
                    statusLine
                        .padding(.top, 12)
                    Spacer(minLength: 0)
                    circle(d)
                    Spacer(minLength: 0)
                    replyBanner
                    controls(vertical: false)
                        .padding(.bottom, 8)
                }
                .padding(.horizontal, 16)
            }
        }
        .onAppear { paneSize = size }
        .onChange(of: size) { _, new in paneSize = new }
    }

    // MARK: The status line (S3.4)

    private var statusLine: some View {
        VStack(spacing: 4) {
            statusMain
            ForEach(statusExtras, id: \.self) { line in
                Text(verbatim: line)
                    .font(.footnote)
                    .multilineTextAlignment(.center)
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Color.black.opacity(0.55), in: RoundedRectangle(cornerRadius: 14))
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var statusMain: some View {
        switch state.phase {
        case .asking:
            Text("Video messages need the camera and the microphone.")
                .font(.callout)
                .multilineTextAlignment(.center)
        case .refused(.camera):
            Text(verbatim: cameraRefusal)
                .font(.callout)
                .multilineTextAlignment(.center)
        case .refused(.microphone):
            Text(verbatim: AudioRecorder.message(for: .microphoneDenied))
                .font(.callout)
                .multilineTextAlignment(.center)
        case .preview:
            if state.firstFrameAtMS == nil && state.problem == nil {
                Text("Starting camera…").font(.callout)
            } else {
                Text("Not recording").font(.callout.weight(.semibold))
            }
        case .recording, .finishing:
            HStack(spacing: 8) {
                Circle()
                    .fill(Color.red)
                    .frame(width: 10, height: 10)
                    .accessibilityHidden(true)
                Text(verbatim: AudioRecorder.timeLabel(Double(state.recordedMS(atMS: session.nowMS)) / 1000))
                    .font(.callout.monospacedDigit().weight(.semibold))
                    // The ticking clock is never announced (S6).
                    .accessibilityHidden(true)
                if state.inWarning(atMS: session.nowMS) {
                    Text("10 seconds left")
                        .font(.callout.weight(.semibold))
                        .foregroundStyle(.orange)
                }
            }
        case .review(let duration):
            Text("Video message · \(AudioRecorder.timeLabel(Double(duration) / 1000))")
                .font(.callout.weight(.semibold))
        case .closed:
            EmptyView()
        }
    }

    /// The lines under the main one.
    private var statusExtras: [String] {
        var lines: [String] = []
        if state.phase == .preview {
            if session.showsFirstTimeLine {
                lines.append(String(localized: "Only you can see this until you start recording."))
            }
            if state.looksDark(atMS: session.nowMS) {
                lines.append(String(localized: "We can't see anything. Is the camera turned off or covered?"))
            }
        }
        if let notice = state.notice {
            lines.append(notice.sentence)
        }
        if case .review = state.phase, state.tooBig {
            lines.append(String(localized: "Too big for a video message. It will be sent as a regular video."))
        }
        return lines
    }

    private var cameraRefusal: String {
        #if os(macOS)
        String(localized: "Family needs permission to use your camera. Turn it on in System Settings › Privacy & Security › Camera.")
        #else
        String(localized: "Family needs permission to use your camera. Turn it on in Settings.")
        #endif
    }

    // MARK: The circle

    /// How far outside the circle its track ring runs, and the progress
    /// over it (the approved design): off the picture, never over a face.
    static let ringOutset: CGFloat = 8

    @ViewBuilder
    private func circle(_ d: CGFloat) -> some View {
        ZStack {
            Circle().fill(Color.white.opacity(0.08))
                .frame(width: d, height: d)
            circleContent(d)
                .frame(width: d, height: d)
                .clipShape(Circle())
            ring(d)
        }
        .frame(width: d + 2 * Self.ringOutset + 4, height: d + 2 * Self.ringOutset + 4)
    }

    @ViewBuilder
    private func circleContent(_ d: CGFloat) -> some View {
        switch state.phase {
        case .preview, .recording, .finishing:
            if let problem = state.problem {
                glyphInCircle("video.slash", text: problem.sentence)
            } else {
                CameraPreviewSurface(layer: session.engine.previewLayer)
                    .accessibilityHidden(true)
            }
        case .review:
            reviewCircle
        case .asking:
            glyphInCircle("video", text: nil)
        case .refused(.camera):
            glyphInCircle("video.slash", text: nil)
        case .refused(.microphone):
            glyphInCircle("mic.slash", text: nil)
        case .closed:
            EmptyView()
        }
    }

    private func glyphInCircle(_ symbol: String, text: String?) -> some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 40))
                .accessibilityHidden(true)
            if let text {
                Text(verbatim: text)
                    .font(.callout)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 20)
            }
        }
        .foregroundStyle(.white.opacity(0.85))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.white.opacity(0.06))
    }

    /// The clip AS IT WILL BE SENT — not mirrored — with a play glyph; a
    /// tap plays it with sound, another pauses (S3.4).
    private var reviewCircle: some View {
        ZStack {
            if let player = session.player {
                RoundVideoSurface(player: player)
            } else {
                Color.white.opacity(0.06)
            }
            if !session.isPlaying {
                Image(systemName: "play.fill")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 44, height: 44)
                    .background(Color.black.opacity(0.45), in: Circle())
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Circle())
        .onTapGesture { session.playPause() }
        .accessibilityElement()
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(session.isPlaying ? Text("Pause") : Text("Play"))
        .accessibilityAction { session.playPause() }
    }

    /// A thin track ring just OUTSIDE the circle, so it never covers a face
    /// (S3.3), and over it the progress: red filling clockwise over the
    /// minute while recording, the tint while the clip plays back in
    /// review. It steps once a second under Reduce Motion (S6).
    @ViewBuilder
    private func ring(_ d: CGFloat) -> some View {
        let frame = d + 2 * Self.ringOutset
        ZStack {
            switch state.phase {
            case .preview, .recording, .finishing, .review:
                Circle()
                    .stroke(Color.white.opacity(0.25), lineWidth: 1)
                    .frame(width: frame, height: frame)
            default:
                EmptyView()
            }
            switch state.phase {
            case .recording, .finishing:
                let recorded = state.recordedMS(atMS: session.nowMS)
                let shown = reduceMotion ? recorded / 1000 * 1000 : recorded
                let progress = state.capMS == 0 ? 1 : min(1, Double(shown) / Double(state.capMS))
                Circle()
                    .trim(from: 0, to: progress)
                    .stroke(ringColor, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .frame(width: frame, height: frame)
            case .review(let duration):
                if session.isPlaying || session.playbackProgress > 0 {
                    let seconds = Double(duration) / 1000
                    let raw = session.playbackProgress
                    let progress = reduceMotion && seconds > 0 ? (raw * seconds).rounded(.down) / seconds : raw
                    Circle()
                        .trim(from: 0, to: progress)
                        .stroke(Color.accentColor, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                        .frame(width: frame, height: frame)
                }
            default:
                EmptyView()
            }
        }
        .accessibilityHidden(true)
    }

    /// Red, filling clockwise from 12 o'clock; orange from the warning —
    /// stronger under Increase Contrast (S6).
    private var ringColor: Color {
        let warning = state.inWarning(atMS: session.nowMS)
        if contrast == .increased {
            return warning ? Color(red: 1, green: 0.55, blue: 0) : Color(red: 1, green: 0.1, blue: 0.1)
        }
        return warning ? .orange : .red
    }

    // MARK: The reply (S3.3)

    @ViewBuilder
    private var replyBanner: some View {
        if session.reply != nil {
            HStack(spacing: 8) {
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(Color.accentColor)
                    .frame(width: 3, height: 32)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    if let title = session.replyTitle {
                        Text(verbatim: title)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.tint)
                    }
                    if let text = session.replyText {
                        Text(verbatim: text)
                            .font(.caption)
                            .foregroundStyle(.white.opacity(0.75))
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                Button {
                    session.dropReply()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.white.opacity(0.7))
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Cancel reply")
                .help("Cancel reply")
            }
            .padding(.leading, 12)
            .frame(maxWidth: 560)
            .background(Color.black.opacity(0.55), in: RoundedRectangle(cornerRadius: 12))
        }
    }

    // MARK: The control row (S3.4)

    @ViewBuilder
    private func controls(vertical: Bool) -> some View {
        let layout = vertical
            ? AnyLayout(VStackLayout(spacing: 14))
            : AnyLayout(HStackLayout(spacing: 14))
        layout {
            leadingControls
            if !vertical { Spacer(minLength: 0) }
            middleControls
            if !vertical { Spacer(minLength: 0) }
            slot
        }
        .frame(maxWidth: vertical ? nil : 560)
    }

    /// Close in the preview, Delete once something is recorded — the
    /// leading place, a small round button with its caption (the approved
    /// design).
    @ViewBuilder
    private var leadingControls: some View {
        switch state.phase {
        case .asking, .refused, .preview:
            roundButton(
                "xmark", label: String(localized: "Close"), caption: String(localized: "Close")
            ) { session.close() }
        case .recording, .finishing:
            roundButton(
                "trash", label: String(localized: "Delete recording"), caption: String(localized: "Delete")
            ) { session.delete() }
        case .review:
            roundButton(
                "trash", label: String(localized: "Delete"), caption: String(localized: "Delete")
            ) { session.delete() }
        case .closed:
            EmptyView()
        }
    }

    /// Switch camera while it previews and records, Retake in review.
    @ViewBuilder
    private var middleControls: some View {
        switch state.phase {
        case .preview:
            cameraChoice
            voiceInsteadButton
        case .refused(.camera):
            settingsButton
            voiceInsteadButton
        case .refused(.microphone):
            settingsButton
        case .recording:
            #if os(iOS)
            // Swapped without a break, iPhone and iPad only (S3.5).
            if session.engine.canSwitchCamera {
                switchCameraButton
            }
            #else
            EmptyView()
            #endif
        case .review:
            roundButton(
                "arrow.counterclockwise", label: String(localized: "Retake"),
                caption: String(localized: "Retake")
            ) { session.retake() }
        default:
            EmptyView()
        }
    }

    #if os(iOS)
    private var switchCameraButton: some View {
        roundButton(
            "arrow.triangle.2.circlepath.camera", label: String(localized: "Switch camera"),
            caption: String(localized: "Switch")
        ) { session.switchCamera() }
    }
    #endif

    @ViewBuilder
    private var cameraChoice: some View {
        #if os(iOS)
        if session.engine.canSwitchCamera {
            switchCameraButton
        }
        #else
        // A desktop with more than one camera chooses among their system
        // names (S3.5).
        let cameras = session.engine.cameras
        if cameras.count > 1 {
            Menu {
                ForEach(cameras) { camera in
                    Button {
                        session.chooseCamera(camera.id)
                    } label: {
                        if camera.id == session.engine.currentCameraID {
                            Label(camera.name, systemImage: "checkmark")
                        } else {
                            Text(verbatim: camera.name)
                        }
                    }
                }
            } label: {
                Image(systemName: "web.camera")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 44, height: 44)
                    .background(Color.white.opacity(Self.smallButtonFill), in: Circle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Choose camera")
            .help("Choose camera")
            .captioned(String(localized: "Switch"))
        }
        #endif
    }

    /// One tap undoes a mis-tap on the video button; dimmed with row 9's
    /// sentence while a not-sent voice message waits (S3.4).
    private var voiceInsteadButton: some View {
        let dimmed = session.request.notSent()
        return roundButton(
            "mic", label: String(localized: "Record a voice message instead"),
            caption: String(localized: "Voice message"), dimmed: dimmed
        ) {
            session.voiceInstead()
        }
        .accessibilityValue(dimmed ? Text(verbatim: ComposerSlot.Dimmed.notSent.notice) : Text(""))
    }

    private var settingsButton: some View {
        Button {
            openSettings()
        } label: {
            #if os(macOS)
            Text("Open System Settings")
            #else
            Text("Open Settings")
            #endif
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
    }

    private func openSettings() {
        #if os(iOS)
        if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
        #else
        let pane = state.phase == .refused(.microphone) ? "Privacy_Microphone" : "Privacy_Camera"
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)") {
            openURL(url)
        }
        #endif
    }

    /// ONE big button in the Send button's place (S3.4, the approved
    /// design): a red disc — Record, dimmed until the first frame — then a
    /// red disc with a white rounded square — Stop — then a disc in the tint
    /// with the Send arrow. 64 across, inside a faint halo, with its caption.
    @ViewBuilder
    private var slot: some View {
        switch state.phase {
        case .preview:
            Button {
                session.record()
            } label: {
                bigDisc(fill: AnyShapeStyle(Color.red)) { EmptyView() }
                    .opacity(state.canRecord ? 1 : 0.4)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Record")
            .help("Record")
            .slotFocus($slotReadFocus, $slotKeyFocus)
            .captioned(String(localized: "Record"))
        case .recording, .finishing:
            Button {
                session.stop()
            } label: {
                bigDisc(fill: AnyShapeStyle(Color.red)) {
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(Color.white)
                        .frame(width: 22, height: 22)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Stop recording")
            .help("Stop recording")
            .slotFocus($slotReadFocus, $slotKeyFocus)
            .captioned(String(localized: "Stop"))
        case .review:
            Button {
                session.send()
            } label: {
                bigDisc(fill: AnyShapeStyle(.tint)) {
                    Image(systemName: "arrow.up")
                        .font(.system(size: 26, weight: .bold))
                        .foregroundStyle(.white)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Send video message")
            .help("Send video message")
            .slotFocus($slotReadFocus, $slotKeyFocus)
            .captioned(String(localized: "Send"))
        default:
            Color.clear.frame(width: Self.bigSide, height: Self.bigSide)
                .captioned(nil)
        }
    }

    /// The big button's side, and the small ones'.
    static let bigSide: CGFloat = 64
    static let smallSide: CGFloat = 44
    static let smallButtonFill: Double = 0.14

    private func bigDisc<Glyph: View>(fill: AnyShapeStyle, @ViewBuilder glyph: () -> Glyph) -> some View {
        ZStack {
            Circle().fill(fill)
            glyph()
        }
        .frame(width: Self.bigSide, height: Self.bigSide)
        // The faint halo that sets it apart from the small buttons.
        .overlay(Circle().strokeBorder(Color.white.opacity(0.18), lineWidth: 4).padding(-4))
        .contentShape(Circle())
    }

    private func roundButton(
        _ symbol: String, label: String, caption: String? = nil, dimmed: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(.white.opacity(dimmed ? 0.4 : 1))
                .frame(width: Self.smallSide, height: Self.smallSide)
                .background(Color.white.opacity(dimmed ? 0.06 : Self.smallButtonFill), in: Circle())
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(verbatim: label))
        .help(Text(verbatim: label))
        .captioned(caption)
    }

    // MARK: Focus and keys

    /// Changes whenever the slot changes, so focus can follow it.
    private var phaseKey: String {
        switch state.phase {
        case .asking: "asking"
        case .refused: "refused"
        case .preview: "preview"
        case .recording: "recording"
        case .finishing: "finishing"
        case .review: "review"
        case .closed: "closed"
        }
    }

    private var questionShown: Binding<Bool> {
        Binding(
            get: { session.state.question != nil },
            set: { shown in
                // Dismissed without an answer (Esc on the alert): Keep.
                if !shown, session.state.question != nil { session.answer(delete: false) }
            })
    }

    private func focusSlot() {
        DispatchQueue.main.async {
            slotReadFocus = true
            slotKeyFocus = true
        }
    }

    #if os(iOS)
    /// A hardware keyboard on iPhone and iPad: Return is the slot, Esc
    /// closes, stops or asks, and in REVIEW Space plays and pauses (S3.4).
    private var keyboardDoors: some View {
        ZStack {
            hiddenKey(.return, title: "Record") { session.activateSlot() }
            hiddenKey(.escape, title: "Close") { session.escape() }
            if case .review = state.phase {
                hiddenKey(.space, title: "Play") { session.playPause() }
            }
        }
        .frame(width: 0, height: 0)
        .accessibilityHidden(true)
    }

    private func hiddenKey(_ key: KeyEquivalent, title: LocalizedStringKey, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .frame(width: 0, height: 0)
                .clipped()
                .opacity(0)
        }
        .buttonStyle(.plain)
        .keyboardShortcut(key, modifiers: [])
        .disabled(state.question != nil)
    }
    #endif
}

private extension View {
    /// The slot is where focus starts and stays (S3.4).
    func slotFocus(_ read: AccessibilityFocusState<Bool>.Binding, _ key: FocusState<Bool>.Binding) -> some View {
        accessibilityFocused(read)
            .focused(key)
    }
}

// MARK: - The live picture

#if os(iOS)
private final class CameraPreviewView: UIView {
    var previewLayer: AVCaptureVideoPreviewLayer? {
        didSet {
            guard oldValue !== previewLayer else { return }
            oldValue?.removeFromSuperlayer()
            if let previewLayer { layer.addSublayer(previewLayer) }
            setNeedsLayout()
        }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        clipsToBounds = true
        isUserInteractionEnabled = false
        isAccessibilityElement = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layoutSubviews() {
        super.layoutSubviews()
        previewLayer?.frame = bounds
        layer.cornerRadius = min(bounds.width, bounds.height) / 2
    }
}

/// The front camera, mirrored, in a circle (S3.4).
private struct CameraPreviewSurface: UIViewRepresentable {
    let layer: AVCaptureVideoPreviewLayer?

    func makeUIView(context: Context) -> CameraPreviewView {
        let view = CameraPreviewView()
        view.previewLayer = layer
        return view
    }

    func updateUIView(_ view: CameraPreviewView, context: Context) {
        view.previewLayer = layer
    }
}
#elseif os(macOS)
private final class CameraPreviewView: NSView {
    var previewLayer: AVCaptureVideoPreviewLayer? {
        didSet {
            guard oldValue !== previewLayer else { return }
            oldValue?.removeFromSuperlayer()
            if let previewLayer { layer?.addSublayer(previewLayer) }
            needsLayout = true
        }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        layer?.masksToBounds = true
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layout() {
        super.layout()
        previewLayer?.frame = bounds
        layer?.cornerRadius = min(bounds.width, bounds.height) / 2
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

private struct CameraPreviewSurface: NSViewRepresentable {
    let layer: AVCaptureVideoPreviewLayer?

    func makeNSView(context: Context) -> CameraPreviewView {
        let view = CameraPreviewView()
        view.previewLayer = layer
        return view
    }

    func updateNSView(_ view: CameraPreviewView, context: Context) {
        view.previewLayer = layer
    }
}

// MARK: - Closing a Mac window over a clip (S4, S8.3)

/// Stands in front of the window's own delegate while the recorder is open,
/// answering `windowShouldClose` and forwarding everything else — SwiftUI
/// offers no way to keep a window from closing.
final class MacWindowCloseGuard: NSObject, NSWindowDelegate {
    var shouldClose: () -> Bool = { true }
    private weak var window: NSWindow?
    /// Held strongly while installed: the window holds its delegate weakly.
    private var original: (any NSWindowDelegate)?

    func install(on window: NSWindow) {
        guard window.delegate !== self else { return }
        original = window.delegate
        self.window = window
        window.delegate = self
    }

    func uninstall() {
        if let window, window.delegate === self { window.delegate = original }
        window = nil
        original = nil
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard shouldClose() else { return false }
        return original?.windowShouldClose?(sender) ?? true
    }

    override func responds(to selector: Selector!) -> Bool {
        super.responds(to: selector) || (original?.responds(to: selector) ?? false)
    }

    override func forwardingTarget(for selector: Selector!) -> Any? {
        if let original, original.responds(to: selector) { return original }
        return super.forwardingTarget(for: selector)
    }
}
#endif

/// A recorder control with its caption under it — words as well as a glyph,
/// in the dim white of the approved design. The caption is the button's own
/// label to a screen reader already, so it is hidden from one. Nil keeps the
/// caption's height, so the three columns line up.
private extension View {
    func captioned(_ caption: String?) -> some View {
        VStack(spacing: 4) {
            self
            RecorderCaption(text: caption)
        }
    }
}

/// The words under a recorder control. They grow with the text size up to
/// `largestType` and no further: the glyphs above them are fixed, and four
/// columns of words at the largest sizes outgrow a phone's width — "Voice
/// message" came out "Voice…" and Record "Rec…". The screen reader reads
/// each button's own label at any size.
struct RecorderCaption: View {
    let text: String?

    static let largestType = DynamicTypeSize.accessibility1

    var body: some View {
        Text(verbatim: text ?? " ")
            .font(.caption2)
            .foregroundStyle(.white.opacity(0.62))
            .lineLimit(1)
            .minimumScaleFactor(0.7)
            .accessibilityHidden(true)
            .dynamicTypeSize(...Self.largestType)
    }
}
