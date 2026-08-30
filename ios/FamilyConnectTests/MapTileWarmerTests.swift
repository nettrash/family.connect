//
//  MapTileWarmerTests.swift
//  FamilyConnectTests
//
//  The map half of "a hidden row still fetches, and draws none of it".
//
//  A hidden location row never builds its `Map`, so without a second road
//  to the same request the tile fetch simply stops when somebody is
//  blocked — and protocol.md names that as an oracle beside the link
//  preview. These are the two rules that decide whether the request goes:
//  the reader's own map setting, and asking once per row rather than once
//  per scroll pass.
//

import Testing
@testable import FamilyConnect

@Suite("Map tile warming")
struct MapTileWarmerTests {

    private func location(_ latitude: Double, _ longitude: Double) -> AttachmentDTO {
        AttachmentDTO(
            id: 1, kind: AttachmentDTO.Kind.location, mime: "application/geo+json",
            size: 0, width: nil, height: nil, durationMS: nil, hasPreview: false,
            name: nil, latitude: latitude, longitude: longitude, accuracyM: 5)
    }

    private func photo(_ id: Int64) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: AttachmentDTO.Kind.photo, mime: "image/jpeg",
            size: 1, width: 100, height: 100, durationMS: nil, hasPreview: true,
            name: nil, latitude: nil, longitude: nil, accuracyM: nil)
    }

    @Test("a hidden location row asks for its tiles")
    func hiddenRowAsks() {
        let points = MapTileWarmer.coordinatesToWarm(
            attachments: [location(59.33, 18.06)],
            mapPreviewsEnabled: true, alreadyWarmed: false)
        #expect(points.count == 1)
        #expect(points.first?.latitude == 59.33)
        #expect(points.first?.longitude == 18.06)
    }

    /// The setting "decides the fetch for every row alike and is never
    /// evaluated per sender" (docs/protocol.md, "Blocking a member").
    /// Somebody with maps off makes no request for a VISIBLE location
    /// either, so making one here would be the leak inverted: the only
    /// device asking Apple about a coordinate it refuses to draw.
    @Test("maps off means no request at all, blocked or not")
    func settingWins() {
        #expect(MapTileWarmer.coordinatesToWarm(
            attachments: [location(59.33, 18.06)],
            mapPreviewsEnabled: false, alreadyWarmed: false).isEmpty)
    }

    /// Once per row. A visible row's map is cached by MapKit after its
    /// first draw, so a hidden row that re-asked on every scroll pass
    /// would be MORE talkative than the visible one it is imitating.
    @Test("a row already warmed does not ask again")
    func asksOnlyOnce() {
        #expect(MapTileWarmer.coordinatesToWarm(
            attachments: [location(59.33, 18.06)],
            mapPreviewsEnabled: true, alreadyWarmed: true).isEmpty)
    }

    /// Only locations carry coordinates; a photo has none and must not
    /// produce a phantom request at (0, 0).
    @Test("rows with nothing to map ask for nothing")
    func nonLocationsAskNothing() {
        #expect(MapTileWarmer.coordinatesToWarm(
            attachments: [photo(1), photo(2)],
            mapPreviewsEnabled: true, alreadyWarmed: false).isEmpty)
        #expect(MapTileWarmer.coordinatesToWarm(
            attachments: [], mapPreviewsEnabled: true, alreadyWarmed: false).isEmpty)
    }

    /// An album can carry more than one, and every one of them is a
    /// request the visible row would have made.
    @Test("every location in the row is asked for")
    func allLocationsAsked() {
        let points = MapTileWarmer.coordinatesToWarm(
            attachments: [location(59.33, 18.06), photo(2), location(-33.87, 151.21)],
            mapPreviewsEnabled: true, alreadyWarmed: false)
        #expect(points.count == 2)
        #expect(points.map(\.latitude) == [59.33, -33.87])
    }
}
