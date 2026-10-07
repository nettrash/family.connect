//
//  PickedMediaPrep.swift
//  FamilyConnect
//
//  One item off the system photo picker (PhotosPicker), made into something
//  `stage` takes — on the iPhone, the iPad and, since #78, the Mac, which
//  gained "Photo or Video" (docs/attachment-menu-2026-10-07.md). Extracted
//  from the phone's composer so the Mac's door is the same code rather than
//  a second copy that drifts: what an item is decided from, which transfer
//  is asked for, and who deletes the movie copy all live here once.
//
//  The composers keep what they own: the sentences a failure is told in,
//  the batch loop and the cap.
//

import CoreTransferable
import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

nonisolated enum PickedMediaPrep {

    /// Why an item could not even be read — the two the composers word
    /// differently. Everything else is `MediaPrep`'s own error, passed on.
    enum Failure: Error, Equatable {
        /// It said it was a video and no movie came across.
        case unreadableVideo
        /// It said it was not a video and no bytes came across.
        case unreadableItem
    }

    /// Prepare one picked photo or video.
    ///
    /// Decided from what the item SAYS it is, rather than trying a movie
    /// transfer and reading the failure as "must be a photo" — a transfer
    /// can fail for reasons that have nothing to do with the kind (iCloud,
    /// cancellation), and that path would then hand a video's bytes to the
    /// photo decoder.
    ///
    /// The picker's movie is a COPY (`PickedMovie`), so when nothing is
    /// made from it — a failure, a refusal, a cancel — it is deleted here.
    /// When `prepareVideo` returns the copy itself as the upload file (the
    /// clip goes as it is), the copy is kept: it is the upload now.
    static func prepare(_ item: PhotosPickerItem, limit: Int) async throws -> MediaPrep.Prepared {
        let isVideo = item.supportedContentTypes.contains { $0.conforms(to: .movie) }
        if isVideo {
            guard let movie = try await item.loadTransferable(type: PickedMovie.self) else {
                throw Failure.unreadableVideo
            }
            do {
                let prepared = try await MediaPrep.prepareVideo(from: movie.url, limit: limit)
                if prepared.fileURL != movie.url {
                    try? FileManager.default.removeItem(at: movie.url)
                }
                return prepared
            } catch {
                try? FileManager.default.removeItem(at: movie.url)
                throw error
            }
        }
        guard let data = try await item.loadTransferable(type: Data.self) else {
            throw Failure.unreadableItem
        }
        return try await MediaPrep.preparePhoto(from: data, limit: limit)
    }
}
