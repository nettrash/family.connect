//
//  VideoMessageRecorder.swift
//  FamilyConnect
//
//  The camera and the file behind a round video message, shared by iPhone,
//  iPad and the Mac (#79, Phase 3 — docs/audio-video-messages-2026-10-04.md,
//  "The recording profile for a round video", S3.4, S3.5, S4, S8).
//
//  ONE `AVCaptureSession` with a video and an audio DATA output feeding an
//  `AVAssetWriter` — never `AVCaptureMovieFileOutput`, which can neither crop
//  to a square, nor leave the file unmirrored while the preview is mirrored,
//  nor open the microphone only at Record. The writer is the profile:
//
//  - MP4, `moov` before `mdat` (`shouldOptimizeForNetworkUse`);
//  - 480 × 480, `ResizeAspectFill` — the centre square of what the camera
//    gives, and from the 640 × 480 mode front cameras offer nothing is
//    upscaled; upright pixels with an identity transform (the capture
//    connection rotates the buffers, `RotationCoordinator`'s angle fixed at
//    Record); NOT mirrored (the data connection's mirroring is off, while the
//    preview layer is mirrored like a mirror);
//  - H.264 High at 500 000 bit/s, a keyframe at most every 2 s, 30 fps at
//    most — a camera delivering fewer is kept as it is;
//  - AAC-LC mono at 64 000 bit/s.
//
//  It is recorded to the profile and uploaded AS RECORDED — never planned
//  again (MediaPlan); RoundClipWriterTests holds the file to exactly that.
//
//  THE MICROPHONE OPENS AT RECORD. The audio input joins the session when
//  recording starts and leaves when it stops, so PREVIEW never lights the
//  system's microphone indicator under a status line that says "Not
//  recording" (S3.4).
//

// AVFoundation's capture types predate Sendable; every one of them here is
// confined to the queue that made it (`CaptureCore`).
@preconcurrency import AVFoundation
import CoreMedia
import Foundation
#if os(iOS)
import UIKit
#endif

// MARK: - The profile

/// The round video's recording profile ("The recording profile for a round
/// video").
nonisolated enum RoundClipProfile {
    static let edge = 480
    static let videoBitrate = 500_000
    static let audioBitrate = 64_000
    static let audioSampleRate = 48_000
    static let maxFrameRate: Double = 30
    static let keyframeSeconds = 2

    /// The video writer's settings.
    static var videoSettings: [String: Any] {
        [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: edge,
            AVVideoHeightKey: edge,
            AVVideoScalingModeKey: AVVideoScalingModeResizeAspectFill,
            // 8-bit SDR, said out loud rather than inherited from a camera
            // format that might be HDR.
            AVVideoColorPropertiesKey: [
                AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
                AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
                AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
            ],
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: videoBitrate,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
                AVVideoMaxKeyFrameIntervalDurationKey: keyframeSeconds,
                AVVideoExpectedSourceFrameRateKey: Int(maxFrameRate),
            ] as [String: Any],
        ]
    }

    /// The audio writer's settings: the voice note's row.
    static var audioSettings: [String: Any] {
        [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVNumberOfChannelsKey: 1,
            AVSampleRateKey: audioSampleRate,
            AVEncoderBitRateKey: audioBitrate,
        ]
    }
}

// MARK: - The picture's shape

/// Which way up the camera's buffers are: wider than tall, or taller than
/// wide. A square buffer is either.
nonisolated enum FrameShape: Equatable, Sendable {
    case landscape
    case portrait

    /// The shape a capture connection delivers when the camera's native
    /// buffers are `native` and the connection turns them by `angle`
    /// degrees: a quarter turn swaps width and height.
    static func delivered(native: FrameShape, rotationAngle angle: CGFloat) -> FrameShape {
        let quarterTurns = Int((angle / 90).rounded()) & 3
        guard quarterTurns & 1 == 1 else { return native }
        return native == .landscape ? .portrait : .landscape
    }

    /// The shape of a `width` × `height` picture; nil for a square.
    static func of(width: Int, height: Int) -> FrameShape? {
        if width > height { return .landscape }
        if height > width { return .portrait }
        return nil
    }

    /// Whether a `width` × `height` buffer is this shape — a square is.
    func matches(width: Int, height: Int) -> Bool {
        Self.of(width: width, height: height).map { $0 == self } ?? true
    }
}

// MARK: - The writer

/// Writes camera sample buffers into a round clip. Not thread-safe: every
/// call comes from ONE serial queue (the capture's data queue, or a test).
nonisolated final class RoundClipWriter: @unchecked Sendable {

    /// What became of one sample buffer.
    nonisolated enum Outcome: Equatable, Sendable {
        case appended
        /// Before the first frame, too soon after the last one (over 30 fps),
        /// or the encoder was not ready — a real-time writer drops rather
        /// than waits.
        case dropped
        /// At or past the length limit; nothing more is taken.
        case limit
    }

    /// What finishing produced.
    nonisolated struct Clip: Equatable, Sendable {
        let url: URL
        let durationMS: UInt64
    }

    let url: URL
    let capMS: UInt64
    /// The shape the capture angle fixed at Record delivers. A frame of the
    /// other shape was turned by the angle BEFORE it — still on its way
    /// when the angle changed — and would be written sideways: it is not
    /// taken, so the clip begins at the first upright frame (S3.4, S3.5).
    /// Nil takes every frame.
    let shape: FrameShape?
    private let writer: AVAssetWriter
    private let videoInput: AVAssetWriterInput
    private let audioInput: AVAssetWriterInput
    private(set) var startTime: CMTime?
    private(set) var lastVideoTime: CMTime?
    private(set) var reachedLimit = false
    private var finished = false

    /// The shortest gap kept between two frames: 1/30 s, less a little for
    /// a camera's jitter.
    private static let minimumFrameGap = 1.0 / RoundClipProfile.maxFrameRate - 0.004

    /// - Parameter realTime: true for a camera, which must never be made to
    ///   wait; a test writing as fast as it can says false.
    init(url: URL, capMS: UInt64, shape: FrameShape? = nil, realTime: Bool = true) throws {
        self.url = url
        self.capMS = capMS
        self.shape = shape
        try? FileManager.default.removeItem(at: url)
        writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        // `moov` before `mdat`, on every client (the profile).
        writer.shouldOptimizeForNetworkUse = true
        videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: RoundClipProfile.videoSettings)
        videoInput.expectsMediaDataInRealTime = realTime
        // Upright pixels, never a rotation matrix (S3.5).
        videoInput.transform = .identity
        audioInput = AVAssetWriterInput(mediaType: .audio, outputSettings: RoundClipProfile.audioSettings)
        audioInput.expectsMediaDataInRealTime = realTime
        guard writer.canAdd(videoInput), writer.canAdd(audioInput) else {
            throw writer.error ?? CocoaError(.fileWriteUnknown)
        }
        writer.add(videoInput)
        writer.add(audioInput)
        guard writer.startWriting() else {
            throw writer.error ?? CocoaError(.fileWriteUnknown)
        }
    }

    var isReadyForVideo: Bool { videoInput.isReadyForMoreMediaData }
    var isReadyForAudio: Bool { audioInput.isReadyForMoreMediaData }
    var failed: Bool { writer.status == .failed }

    /// The clip begins at the first frame after Record (S3.4).
    func appendVideo(_ sample: CMSampleBuffer) -> Outcome {
        guard !finished, writer.status == .writing else { return .dropped }
        if let shape, let pixels = CMSampleBufferGetImageBuffer(sample),
           !shape.matches(width: CVPixelBufferGetWidth(pixels), height: CVPixelBufferGetHeight(pixels)) {
            return .dropped
        }
        let time = CMSampleBufferGetPresentationTimeStamp(sample)
        if let start = startTime {
            if elapsedMS(time, from: start) >= capMS {
                reachedLimit = true
                return .limit
            }
            if let last = lastVideoTime, CMTimeGetSeconds(CMTimeSubtract(time, last)) < Self.minimumFrameGap {
                return .dropped
            }
        } else {
            writer.startSession(atSourceTime: time)
            startTime = time
        }
        guard videoInput.isReadyForMoreMediaData, videoInput.append(sample) else { return .dropped }
        lastVideoTime = time
        return .appended
    }

    /// Sound before the first frame is not part of the clip.
    func appendAudio(_ sample: CMSampleBuffer) -> Outcome {
        guard !finished, writer.status == .writing, let start = startTime else { return .dropped }
        let time = CMSampleBufferGetPresentationTimeStamp(sample)
        guard CMTimeCompare(time, start) >= 0 else { return .dropped }
        if elapsedMS(time, from: start) >= capMS { return .limit }
        guard audioInput.isReadyForMoreMediaData, audioInput.append(sample) else { return .dropped }
        return .appended
    }

    /// Close the file. Nil when there is nothing readable: no frame was
    /// ever written, or the writer failed.
    func finish(_ done: @escaping @Sendable (Clip?) -> Void) {
        guard !finished else { return }
        finished = true
        guard let start = startTime, let last = lastVideoTime, writer.status == .writing else {
            writer.cancelWriting()
            try? FileManager.default.removeItem(at: url)
            done(nil)
            return
        }
        // The last frame lasts one frame; the sound stops with the picture.
        let end = CMTimeAdd(last, CMTime(value: 1, timescale: CMTimeScale(RoundClipProfile.maxFrameRate)))
        videoInput.markAsFinished()
        audioInput.markAsFinished()
        writer.endSession(atSourceTime: end)
        let url = url
        let durationMS = elapsedMS(end, from: start)
        writer.finishWriting { [self] in
            if writer.status == .completed {
                done(Clip(url: url, durationMS: durationMS))
            } else {
                try? FileManager.default.removeItem(at: url)
                done(nil)
            }
        }
    }

    /// Throw the take away.
    func cancel() {
        guard !finished else { return }
        finished = true
        writer.cancelWriting()
        try? FileManager.default.removeItem(at: url)
    }

    private func elapsedMS(_ time: CMTime, from start: CMTime) -> UInt64 {
        let seconds = CMTimeGetSeconds(CMTimeSubtract(time, start))
        guard seconds.isFinite, seconds > 0 else { return 0 }
        return UInt64(seconds * 1000)
    }
}

// MARK: - What the session drives

/// One camera the recorder can use, for the Mac's "Choose camera".
nonisolated struct VideoCameraChoice: Equatable, Identifiable, Sendable {
    let id: String
    let name: String
}

/// What the camera says, on the main actor.
nonisolated enum VideoCaptureEvent: Equatable, Sendable {
    case firstFrame
    /// A frame sampled about four times a second, and whether it is
    /// near-black (S3.6).
    case frame(dark: Bool)
    case problem(VideoRecorderMachine.CameraProblem?)
    case microphoneTaken
    case limitReached
    case failed
    case finished(RoundClipWriter.Clip?)
}

/// The camera, as the session sees it. The app gets `VideoMessageRecorder`;
/// a test gets a fake that says what it is told to.
@MainActor
protocol VideoCaptureEngine: AnyObject {
    var onEvent: ((VideoCaptureEvent) -> Void)? { get set }
    /// The live picture; nil in a fake.
    var previewLayer: AVCaptureVideoPreviewLayer? { get }
    /// Every camera there is (the Mac's "Choose camera" when more than one).
    var cameras: [VideoCameraChoice] { get }
    var currentCameraID: String? { get }
    /// Front ↔ back, on a phone or a tablet.
    var canSwitchCamera: Bool { get }
    func startPreview()
    func stopPreview()
    func startRecording(to url: URL, capMS: UInt64)
    func stopRecording()
    func cancelRecording()
    func switchCamera()
    func chooseCamera(_ id: String)
}

// MARK: - The camera

/// The real camera: an `AVCaptureSession` and the writer above.
@MainActor
final class VideoMessageRecorder: VideoCaptureEngine {

    var onEvent: ((VideoCaptureEvent) -> Void)?
    let previewLayer: AVCaptureVideoPreviewLayer?
    private let core: CaptureCore
    private var observers: [any NSObjectProtocol] = []
    #if os(iOS)
    private var rotation: AVCaptureDevice.RotationCoordinator?
    private var rotationObservation: NSKeyValueObservation?
    private var pressureObservation: NSKeyValueObservation?
    private var underPressure = false
    #endif

    /// Whether this device has a camera at all — S1.2's **round available**.
    ///
    /// Read by every composer redraw — every keystroke — so the answer is
    /// kept, and asked again only after a camera comes or goes.
    static var hasCamera: Bool {
        if let knownHasCamera { return knownHasCamera }
        watchCamerasComingAndGoing()
        let found = lookForCamera()
        knownHasCamera = found
        return found
    }

    /// The system's answer; a test counts how often it is asked.
    static var lookForCamera: () -> Bool = {
        #if os(macOS)
        AVCaptureDevice.systemPreferredCamera != nil || AVCaptureDevice.default(for: .video) != nil
        #else
        AVCaptureDevice.default(for: .video) != nil
        #endif
    }

    private static var knownHasCamera: Bool?
    private static var cameraWatchers: [any NSObjectProtocol] = []

    /// Forget the kept answer: a camera came or went.
    static func camerasChanged() {
        knownHasCamera = nil
    }

    private static func watchCamerasComingAndGoing() {
        guard cameraWatchers.isEmpty else { return }
        let center = NotificationCenter.default
        for name in [AVCaptureDevice.wasConnectedNotification, AVCaptureDevice.wasDisconnectedNotification] {
            cameraWatchers.append(center.addObserver(forName: name, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { camerasChanged() }
            })
        }
    }

    init() {
        let core = CaptureCore()
        self.core = core
        previewLayer = AVCaptureVideoPreviewLayer(session: core.session)
        previewLayer?.videoGravity = .resizeAspectFill
        core.emit = { [weak self] event in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.received(event) }
            }
        }
        watchTheSession()
    }

    deinit {
        for token in observers { NotificationCenter.default.removeObserver(token) }
    }

    var cameras: [VideoCameraChoice] {
        Self.discovery().devices.map { VideoCameraChoice(id: $0.uniqueID, name: $0.localizedName) }
    }

    var currentCameraID: String? { core.currentDeviceID }

    var canSwitchCamera: Bool {
        #if os(iOS)
        Self.discovery().devices.contains { $0.position == .back }
            && Self.discovery().devices.contains { $0.position == .front }
        #else
        false
        #endif
    }

    func startPreview() {
        let device = Self.startingDevice()
        core.start(device: device) { [weak self] in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.cameraChanged() }
            }
        }
    }

    func stopPreview() {
        core.stop()
    }

    func startRecording(to url: URL, capMS: UInt64) {
        core.beginRecording(url: url, capMS: capMS, angle: captureAngle())
    }

    func stopRecording() {
        core.endRecording(cancel: false)
    }

    func cancelRecording() {
        core.endRecording(cancel: true)
    }

    func switchCamera() {
        #if os(iOS)
        let devices = Self.discovery().devices
        let current = devices.first { $0.uniqueID == core.currentDeviceID }
        let wanted: AVCaptureDevice.Position = current?.position == .front ? .back : .front
        guard let next = devices.first(where: { $0.position == wanted }) else { return }
        core.replaceDevice(next) { [weak self] in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.cameraChanged() } }
        }
        #endif
    }

    func chooseCamera(_ id: String) {
        guard let device = Self.discovery().devices.first(where: { $0.uniqueID == id }) else { return }
        AppSettings.videoMessageCameraID = id
        #if os(macOS)
        AVCaptureDevice.userPreferredCamera = device
        #endif
        core.replaceDevice(device) { [weak self] in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.cameraChanged() } }
        }
    }

    // MARK: - Which camera

    private static func discovery() -> AVCaptureDevice.DiscoverySession {
        #if os(iOS)
        AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera], mediaType: .video, position: .unspecified)
        #else
        AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external, .continuityCamera],
            mediaType: .video, position: .unspecified)
        #endif
    }

    /// The front camera first, any camera otherwise; the Mac starts from
    /// `systemPreferredCamera` (Continuity Camera included). A choice made
    /// on this device is remembered (S3.5).
    private static func startingDevice() -> AVCaptureDevice? {
        let devices = discovery().devices
        if let chosen = AppSettings.videoMessageCameraID,
           let device = devices.first(where: { $0.uniqueID == chosen }) {
            return device
        }
        #if os(iOS)
        return AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
            ?? AVCaptureDevice.default(for: .video)
        #else
        return AVCaptureDevice.systemPreferredCamera ?? AVCaptureDevice.default(for: .video)
        #endif
    }

    // MARK: - The picture's angle

    /// After every change of camera: the preview mirrored like a mirror, and
    /// (iPhone, iPad) a rotation coordinator for the new device.
    private func cameraChanged() {
        guard let connection = previewLayer?.connection else { return }
        #if os(macOS)
        // A desktop's self-view is a mirror, as FaceTime's is.
        if connection.isVideoMirroringSupported {
            connection.automaticallyAdjustsVideoMirroring = false
            connection.isVideoMirrored = true
        }
        #else
        if let device = core.currentDevice {
            let coordinator = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: previewLayer)
            rotation = coordinator
            applyPreviewAngle(coordinator.videoRotationAngleForHorizonLevelPreview)
            rotationObservation = coordinator.observe(
                \.videoRotationAngleForHorizonLevelPreview, options: [.new]
            ) { [weak self] coordinator, _ in
                let angle = coordinator.videoRotationAngleForHorizonLevelPreview
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.applyPreviewAngle(angle) }
                }
            }
            // Under pressure the session is interrupted until it abates (S4);
            // only reaching and leaving `.shutdown` is said.
            pressureObservation = device.observe(\.systemPressureState, options: [.new]) { [weak self] device, _ in
                let shutdown = device.systemPressureState.level == .shutdown
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.pressureChanged(shutdown: shutdown) }
                }
            }
        }
        #endif
    }

    #if os(iOS)
    private func pressureChanged(shutdown: Bool) {
        guard shutdown != underPressure else { return }
        underPressure = shutdown
        onEvent?(.problem(shutdown ? .unavailable : nil))
    }

    private func applyPreviewAngle(_ angle: CGFloat) {
        guard let connection = previewLayer?.connection, connection.isVideoRotationAngleSupported(angle) else {
            return
        }
        connection.videoRotationAngle = angle
    }
    #endif

    /// The angle the file is written at, fixed at Record (S3.5).
    private func captureAngle() -> CGFloat? {
        #if os(iOS)
        rotation?.videoRotationAngleForHorizonLevelCapture
        #else
        nil
        #endif
    }

    // MARK: - Interruptions (S4, S3.6)

    private func received(_ event: VideoCaptureEvent) {
        onEvent?(event)
    }

    private func watchTheSession() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: AVCaptureSession.wasInterruptedNotification, object: core.session, queue: .main
        ) { [weak self] note in
            let problem = Self.problem(from: note)
            MainActor.assumeIsolated { self?.interrupted(problem) }
        })
        observers.append(center.addObserver(
            forName: AVCaptureSession.interruptionEndedNotification, object: core.session, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.onEvent?(.problem(nil)) }
        })
        observers.append(center.addObserver(
            forName: AVCaptureSession.runtimeErrorNotification, object: core.session, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                if self.core.isRecording {
                    self.onEvent?(.failed)
                } else {
                    self.onEvent?(.problem(.unavailable))
                }
            }
        })
        // A USB webcam pulled, a Continuity Camera iPhone taken away — how
        // each surfaces is UNCONFIRMED (S4); a disconnect is the one AVFoundation
        // names.
        observers.append(center.addObserver(
            forName: AVCaptureDevice.wasDisconnectedNotification, object: nil, queue: .main
        ) { [weak self] note in
            let gone = (note.object as? AVCaptureDevice)?.uniqueID
            MainActor.assumeIsolated {
                guard let self, gone != nil, gone == self.core.currentDeviceID else { return }
                self.onEvent?(.problem(.unavailable))
            }
        })
    }

    private enum Interruption: Sendable {
        case camera(VideoRecorderMachine.CameraProblem)
        case microphone
        case ignore
    }

    /// Which S3.6 sentence an interruption is.
    nonisolated private static func problem(from note: Notification) -> Interruption {
        #if os(iOS)
        guard let raw = note.userInfo?[AVCaptureSessionInterruptionReasonKey] as? Int,
              let reason = AVCaptureSession.InterruptionReason(rawValue: raw)
        else { return .camera(.unavailable) }
        switch reason {
        case .videoDeviceInUseByAnotherClient: return .camera(.inUse)
        case .videoDeviceNotAvailableWithMultipleForegroundApps: return .camera(.multitasking)
        case .audioDeviceInUseByAnotherClient: return .microphone
        // The background is the app's own business: the recorder closes or
        // stops into REVIEW through `wentAway`, never twice.
        case .videoDeviceNotAvailableInBackground: return .ignore
        default: return .camera(.unavailable)
        }
        #else
        return .camera(.unavailable)
        #endif
    }

    private func interrupted(_ interruption: Interruption) {
        switch interruption {
        case .camera(let problem): onEvent?(.problem(problem))
        case .microphone: onEvent?(.microphoneTaken)
        case .ignore: break
        }
    }
}

// MARK: - The session's own queues

/// Everything that runs off the main actor: the session's configuration on
/// one queue, the sample buffers and the writer on another.
nonisolated private final class CaptureCore: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate,
    AVCaptureAudioDataOutputSampleBufferDelegate, @unchecked Sendable
{
    let session = AVCaptureSession()
    var emit: @Sendable (VideoCaptureEvent) -> Void = { _ in }

    private let sessionQueue = DispatchQueue(label: "me.nettrash.FamilyConnect.video.session")
    private let dataQueue = DispatchQueue(label: "me.nettrash.FamilyConnect.video.data")
    private let videoOutput = AVCaptureVideoDataOutput()
    private let audioOutput = AVCaptureAudioDataOutput()
    private var videoInput: AVCaptureDeviceInput?
    private var audioInput: AVCaptureDeviceInput?
    private var configured = false
    /// The capture angle fixed at Record, re-applied after a camera swap.
    private var fixedAngle: CGFloat?

    // Guarded by `lock`: read from the main actor.
    private let lock = NSLock()
    private var _deviceID: String?
    private var _device: AVCaptureDevice?
    private var _recording = false

    // The data queue's own.
    private var writer: RoundClipWriter?
    private var sentFirstFrame = false
    private var sentLimit = false
    private var lastSampleAt: CMTime?

    var currentDeviceID: String? { lock.withLock { _deviceID } }
    var currentDevice: AVCaptureDevice? { lock.withLock { _device } }
    var isRecording: Bool { lock.withLock { _recording } }

    // MARK: Camera on, camera off

    func start(device: AVCaptureDevice?, ready: @escaping @Sendable () -> Void) {
        dataQueue.async { [self] in
            sentFirstFrame = false
            lastSampleAt = nil
        }
        sessionQueue.async { [self] in
            if !configured { configure(device: device) }
            // Off the main thread: `startRunning` blocks until it runs.
            if !session.isRunning { session.startRunning() }
            ready()
        }
    }

    func stop() {
        sessionQueue.async { [self] in
            if session.isRunning { session.stopRunning() }
        }
    }

    private func configure(device: AVCaptureDevice?) {
        session.beginConfiguration()
        defer { session.commitConfiguration() }
        configured = true
        if session.canSetSessionPreset(.vga640x480) {
            session.sessionPreset = .vga640x480
        } else if session.canSetSessionPreset(.medium) {
            session.sessionPreset = .medium
        }
        #if os(iOS)
        // iPad multitasking: no entitlement at a 17.0 deployment target (S3.5).
        if session.isMultitaskingCameraAccessSupported {
            session.isMultitaskingCameraAccessEnabled = true
        }
        // The session sets `.playAndRecord` itself when the audio input joins
        // at Record — and only then.
        session.automaticallyConfiguresApplicationAudioSession = true
        #endif
        if let device {
            attach(device)
        } else {
            // No camera after all: said as the camera being unavailable.
            emit(.problem(.unavailable))
        }
        videoOutput.alwaysDiscardsLateVideoFrames = true
        let wanted = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        if videoOutput.availableVideoPixelFormatTypes.contains(wanted) {
            videoOutput.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: wanted]
        }
        videoOutput.setSampleBufferDelegate(self, queue: dataQueue)
        if session.canAddOutput(videoOutput) { session.addOutput(videoOutput) }
        audioOutput.setSampleBufferDelegate(self, queue: dataQueue)
        configureVideoConnection()
    }

    private func attach(_ device: AVCaptureDevice) {
        if let videoInput { session.removeInput(videoInput) }
        guard let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) else {
            videoInput = nil
            lock.withLock { _deviceID = nil; _device = nil }
            emit(.problem(.unavailable))
            return
        }
        session.addInput(input)
        videoInput = input
        lock.withLock { _deviceID = device.uniqueID; _device = device }
        // 30 fps at most; fewer stays as it is (the profile).
        if (try? device.lockForConfiguration()) != nil {
            let thirty = CMTime(value: 1, timescale: 30)
            let fastest = device.activeFormat.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
            if fastest >= 30 { device.activeVideoMinFrameDuration = thirty }
            device.unlockForConfiguration()
        }
    }

    /// The FILE is the true view, never mirrored (S3.5) — and, from Record,
    /// written at the angle fixed then.
    private func configureVideoConnection() {
        guard let connection = videoOutput.connection(with: .video) else { return }
        if connection.isVideoMirroringSupported {
            connection.automaticallyAdjustsVideoMirroring = false
            connection.isVideoMirrored = false
        }
        if let fixedAngle, connection.isVideoRotationAngleSupported(fixedAngle) {
            connection.videoRotationAngle = fixedAngle
        }
    }

    func replaceDevice(_ device: AVCaptureDevice, ready: @escaping @Sendable () -> Void) {
        sessionQueue.async { [self] in
            session.beginConfiguration()
            attach(device)
            configureVideoConnection()
            session.commitConfiguration()
            ready()
        }
    }

    // MARK: Recording

    /// The angle and the microphone are committed FIRST; only then is the
    /// writer made, on the data queue behind every frame already delivered
    /// at the preview's angle — so no frame from before the angle took hold
    /// is written, and the clip begins at the first upright frame after
    /// Record (S3.4, S3.5). The writer also refuses a frame of the wrong
    /// shape, for one still in flight when the angle changed.
    func beginRecording(url: URL, capMS: UInt64, angle: CGFloat?) {
        lock.withLock { _recording = true }
        sessionQueue.async { [self] in
            session.beginConfiguration()
            fixedAngle = angle
            configureVideoConnection()
            // The microphone opens now, and not before (S3.4).
            if audioInput == nil, let microphone = AVCaptureDevice.default(for: .audio),
               let input = try? AVCaptureDeviceInput(device: microphone), session.canAddInput(input) {
                session.addInput(input)
                audioInput = input
            }
            if !session.outputs.contains(audioOutput), session.canAddOutput(audioOutput) {
                session.addOutput(audioOutput)
            }
            session.commitConfiguration()
            let shape = deliveredShape()
            dataQueue.async { [self] in
                sentLimit = false
                do {
                    writer = try RoundClipWriter(url: url, capMS: capMS, shape: shape)
                } catch {
                    writer = nil
                    emit(.failed)
                }
            }
        }
    }

    /// The shape the data connection now delivers, when an angle was fixed
    /// and took hold; nil (every frame taken) otherwise — the Mac, or an
    /// angle the connection does not support.
    private func deliveredShape() -> FrameShape? {
        guard let fixedAngle, let connection = videoOutput.connection(with: .video),
              connection.videoRotationAngle == fixedAngle,
              let device = lock.withLock({ _device })
        else { return nil }
        let dimensions = CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription)
        guard let native = FrameShape.of(width: Int(dimensions.width), height: Int(dimensions.height)) else {
            return nil
        }
        return FrameShape.delivered(native: native, rotationAngle: fixedAngle)
    }

    /// Through the session queue, as `beginRecording` is, so a Stop can never
    /// overtake the writer it stops: the finish is queued on the data queue
    /// behind the writer's making, and before the microphone is let go.
    func endRecording(cancel: Bool) {
        lock.withLock { _recording = false }
        sessionQueue.async { [self] in
            dataQueue.async { [self] in
                let finishing = writer
                writer = nil
                if cancel {
                    finishing?.cancel()
                } else if let finishing {
                    finishing.finish { [emit] clip in emit(.finished(clip)) }
                } else {
                    emit(.finished(nil))
                }
            }
            session.beginConfiguration()
            fixedAngle = nil
            if let audioInput { session.removeInput(audioInput) }
            audioInput = nil
            if session.outputs.contains(audioOutput) { session.removeOutput(audioOutput) }
            session.commitConfiguration()
        }
    }

    // MARK: Samples

    func captureOutput(
        _ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection
    ) {
        if output === audioOutput {
            _ = writer?.appendAudio(sampleBuffer)
            return
        }
        if !sentFirstFrame {
            sentFirstFrame = true
            emit(.firstFrame)
        }
        let time = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        if writer == nil {
            // Brightness, four times a second, for "We can't see anything".
            if lastSampleAt == nil || CMTimeGetSeconds(CMTimeSubtract(time, lastSampleAt!)) >= 0.25,
               let pixels = CMSampleBufferGetImageBuffer(sampleBuffer),
               let luma = Self.meanLuma(pixels) {
                lastSampleAt = time
                emit(.frame(dark: NearBlack.isDark(meanLuma: luma)))
            }
            return
        }
        guard let writer else { return }
        if writer.appendVideo(sampleBuffer) == .limit, !sentLimit {
            sentLimit = true
            emit(.limitReached)
        }
        if writer.failed {
            self.writer = nil
            writer.cancel()
            emit(.failed)
        }
    }

    /// The mean of every 8th luma sample of every 8th row.
    static func meanLuma(_ buffer: CVPixelBuffer) -> Double? {
        let format = CVPixelBufferGetPixelFormatType(buffer)
        let videoRange = format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        guard videoRange || format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange else { return nil }
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0) else { return nil }
        let width = CVPixelBufferGetWidthOfPlane(buffer, 0)
        let height = CVPixelBufferGetHeightOfPlane(buffer, 0)
        let rowBytes = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
        let bytes = base.assumingMemoryBound(to: UInt8.self)
        var sum: UInt64 = 0
        var count = 0
        var row = 0
        while row < height {
            var column = 0
            while column < width {
                sum += UInt64(bytes[row * rowBytes + column])
                count += 1
                column += 8
            }
            row += 8
        }
        return NearBlack.meanLuma(sum: sum, count: count, videoRange: videoRange)
    }
}
