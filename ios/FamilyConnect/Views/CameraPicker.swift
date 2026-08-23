//
//  CameraPicker.swift
//  FamilyConnect
//
//  Take a photo or shoot a video without leaving the chat.
//
//  `UIImagePickerController` rather than a hand-built `AVCaptureSession`:
//  SwiftUI has no capture counterpart to `PhotosPicker` (that one is
//  library-only), and rolling our own would mean building preview, shutter,
//  flip, flash, focus, zoom and rotation handling for no gain. The picker's
//  *library* source types are the soft-deprecated ones — `.camera` is not.
//
//  One entry covers both stills and clips: handing it both media types gives
//  the picker its own PHOTO/VIDEO toggle, which is the control people already
//  know, so the attach menu needs one "Camera" item rather than two.
//
//  iOS only. The Mac keeps its single open panel — a webcam is not how anyone
//  sends a photo from a desktop, and adding capture there would mean a camera
//  entitlement the sandboxed app deliberately does not have.
//

#if os(iOS)

import AVFoundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct CameraPicker: UIViewControllerRepresentable {
    /// A still, already encoded — the camera hands back a `UIImage`, not a
    /// file, which is the one place this differs from every other source.
    var onPhoto: (Data) -> Void
    /// A clip, as a real file in tmp.
    var onVideo: (URL) -> Void

    @Environment(\.dismiss) private var dismiss

    /// Whether this device can actually capture. False in the Simulator and
    /// on a camera-less device, and the menu entry is hidden when it is —
    /// presenting the picker anyway shows an empty black sheet.
    static var isAvailable: Bool {
        UIImagePickerController.isSourceTypeAvailable(.camera)
    }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = [UTType.image.identifier, UTType.movie.identifier]
        picker.videoQuality = .typeHigh
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator {
        Coordinator(onPhoto: onPhoto, onVideo: onVideo, dismiss: { dismiss() })
    }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate,
                             UINavigationControllerDelegate {
        private let onPhoto: (Data) -> Void
        private let onVideo: (URL) -> Void
        private let dismiss: () -> Void

        init(onPhoto: @escaping (Data) -> Void,
             onVideo: @escaping (URL) -> Void,
             dismiss: @escaping () -> Void) {
            self.onPhoto = onPhoto
            self.onVideo = onVideo
            self.dismiss = dismiss
        }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            // Video first: a movie always carries a URL, and asking about the
            // URL is a fact rather than an inference about which mode was used.
            if let url = info[.mediaURL] as? URL {
                dismiss()
                onVideo(url)
                return
            }
            if let image = info[.originalImage] as? UIImage {
                // The capture is already oriented by `UIImage.imageOrientation`
                // rather than an EXIF tag, and JPEG encoding here bakes that in
                // — skipping it lands portrait shots on their side.
                let upright = image.upright()
                if let data = upright.jpegData(compressionQuality: 0.9) {
                    dismiss()
                    onPhoto(data)
                    return
                }
            }
            dismiss()
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            dismiss()
        }
    }
}

private extension UIImage {
    /// Redraw with the orientation applied, so the pixels are the picture.
    func upright() -> UIImage {
        guard imageOrientation != .up else { return self }
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = scale
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            draw(in: CGRect(origin: .zero, size: size))
        }
    }
}

#endif
