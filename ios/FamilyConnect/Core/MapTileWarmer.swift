//
//  MapTileWarmer.swift
//  FamilyConnect
//
//  The map half of "a hidden row still fetches, and draws none of it".
//
//  A visible location bubble asks Apple for tiles the moment MapKit draws
//  it. A HIDDEN one never builds that view — SwiftUI only realises the
//  branch it takes — so without this the tile request simply stops, and
//  protocol.md ("Blocking a member") names that as an oracle in the same
//  breath as the link preview: "A client resolves a hidden message's link
//  preview, ITS MAP TILES, its attachment preview and its sender's avatar
//  exactly as it would for a visible row, on the same schedule, and
//  discards every result until the row is revealed."
//
//  So the hidden path asks by another road. `MKMapSnapshotter` makes the
//  same tile request the Map view would and hands back an image nobody
//  looks at, which is the cheaper half of the trade the protocol makes.
//
//  Two things it deliberately inherits from the drawn map, because a
//  request that does not MATCH is its own signal: the region is the same
//  ~600 m span LocationAttachmentView uses, and the size is the same
//  260×140 the bubble draws.
//

import Foundation
import MapKit

enum MapTileWarmer {
    /// The map bubble's own geometry — kept here and in
    /// `LocationAttachmentView` deliberately: if the drawn map changes
    /// span or size, the warm request has to change with it.
    private static let span = MKCoordinateSpan(latitudeDelta: 0.005, longitudeDelta: 0.005)
    private static let size = CGSize(width: 260, height: 140)

    /// Rows already warmed, so a row rebuilt by a scroll does not re-ask.
    /// A visible row's map is cached by MapKit after its first draw; this
    /// keeps the hidden path to the same one-request-per-row shape rather
    /// than a request per scroll pass.
    @MainActor private static var warmed: Set<String> = []

    /// Ask for the tiles a hidden row would have drawn, and throw them
    /// away.
    ///
    /// Gated on `mapPreviewsEnabled` because the setting "decides the
    /// fetch for every row alike and is never evaluated per sender"
    /// (protocol.md). Somebody with maps off makes no map request for a
    /// visible location either, so making one here would be the leak
    /// inverted — the only device on the family's network asking Apple
    /// about a coordinate it refuses to draw.
    /// WHICH coordinates this row should ask for, separated from the
    /// asking so the rule can be tested. `MKMapSnapshotter` reports
    /// nothing a test can see — it either contacts Apple or it does not —
    /// so a `warm` that both decided and fired would leave the two rules
    /// that matter (the setting, and asking once) resting on inspection.
    ///
    /// Pure, and takes `alreadyWarmed` rather than reading the store, so a
    /// test can ask about the second visit without staging the first.
    nonisolated static func coordinatesToWarm(
        attachments: [AttachmentDTO],
        mapPreviewsEnabled: Bool,
        alreadyWarmed: Bool
    ) -> [(latitude: Double, longitude: Double)] {
        guard mapPreviewsEnabled, !alreadyWarmed else { return [] }
        return attachments.compactMap(\.coordinate)
    }

    @MainActor
    static func warm(localID: String, attachments: [AttachmentDTO]) {
        let points = coordinatesToWarm(
            attachments: attachments,
            mapPreviewsEnabled: AppSettings.mapPreviewsEnabled,
            alreadyWarmed: warmed.contains(localID))
        guard !points.isEmpty else { return }
        warmed.insert(localID)
        for point in points {
            let options = MKMapSnapshotter.Options()
            options.region = MKCoordinateRegion(
                center: CLLocationCoordinate2D(latitude: point.latitude, longitude: point.longitude),
                span: span)
            options.size = size
            // The result is discarded on purpose. The REQUEST is the whole
            // point; the image is what a visible row would have shown.
            MKMapSnapshotter(options: options).start { _, _ in }
        }
    }

    /// Testing seam: the warm set is process-wide, so a test that asserts
    /// on first-ask behaviour has to be able to clear it.
    @MainActor
    static func resetForTesting() { warmed.removeAll() }
}
