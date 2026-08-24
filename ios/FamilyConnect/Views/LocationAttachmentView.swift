//
//  LocationAttachmentView.swift
//  FamilyConnect
//
//  A shared place, in a bubble.
//
//  Platform-free on purpose — iOS and macOS draw the same thing, from the
//  same coordinates, with MapKit on both. There are no bytes to fetch: a
//  location's whole content arrives on the attachment (docs/protocol.md,
//  "Locations"), so this draws immediately and works offline for anything
//  already in the cache.
//
//  THE MAP IS DRAWN BY THIS DEVICE, and that is the privacy trade the
//  feature makes: rendering tiles means asking Apple for them, with the
//  coordinate a family member deliberately sent. It is the same trade the
//  link previews make, so it takes the same shape — a Settings switch, and
//  no contact at all when it is off. With the map off the bubble still
//  shows the pin, the label and a way into the system map app, which is a
//  hand-off the person chooses rather than a request the app makes.
//

import MapKit
import SwiftUI

struct LocationAttachmentView: View {
    let attachment: AttachmentDTO
    /// The bubble's own gestures, which a child must not swallow — the
    /// same pass-through every other attachment block does.
    var onLongPress: () -> Void = {}
    var onDoubleTap: () -> Void = {}
    let isMine: Bool

    @Environment(\.openURL) private var openURL

    private var coordinate: CLLocationCoordinate2D? {
        guard let point = attachment.coordinate else { return nil }
        return CLLocationCoordinate2D(latitude: point.latitude, longitude: point.longitude)
    }

    var body: some View {
        // The row is drawn even with NO coordinate, and that is a
        // deliberate floor rather than defensive noise. A location has no
        // bytes to fall back on, so anything that loses the pin — a sync
        // bug, a row written by an older build — used to render an
        // absolutely EMPTY balloon, which reads as the app being broken
        // rather than as one missing detail. Saying "Location" and nothing
        // else is a bad outcome; saying nothing at all is a worse one.
        VStack(alignment: .leading, spacing: 6) {
            if let coordinate, AppSettings.mapPreviewsEnabled {
                map(coordinate)
            }
            label()
        }
        .frame(maxWidth: 260, alignment: .leading)
        // Count 2 BEFORE count 1: that is what makes them exclusive. A
        // bare `simultaneousGesture` means "recognise alongside", so the
        // single tap opens Maps before the double tap can land the heart —
        // the exact bug the attachment blocks already fixed.
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture { if let coordinate { open(coordinate) } }
        .onLongPressGesture { onLongPress() }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isButton)
        // A bare gesture publishes NO accessibility action — measured in
        // this repo, on this exact pattern. Without this VoiceOver
        // announces a button that does nothing.
        .accessibilityAction { if let coordinate { open(coordinate) } }
    }

    /// No caption on the pin. See the note at the call site.
    private let unlabelled = ""

    private func map(_ coordinate: CLLocationCoordinate2D) -> some View {
        Map(
            initialPosition: .region(
                MKCoordinateRegion(
                    center: coordinate,
                    // ~600 m across: close enough to recognise the street,
                    // wide enough to place it in a neighbourhood.
                    span: MKCoordinateSpan(latitudeDelta: 0.005, longitudeDelta: 0.005))),
            // Not interactive inside a bubble: a scrolling thread and a
            // pannable map fight over every drag, and the map wins, which
            // makes the conversation feel stuck.
            interactionModes: []
        ) {
            // A String VARIABLE, not a literal: `Marker(""…)` takes a
            // LocalizedStringKey, so the empty literal was extracted into
            // the string catalogue as an empty key needing eight
            // translations. The pin wants no label at all — the name is
            // already on the row underneath it.
            Marker(unlabelled, coordinate: coordinate)
                .tint(.red)
        }
        .frame(height: 140)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .allowsHitTesting(false)
    }

    private func label() -> some View {
        HStack(spacing: 8) {
            Image(systemName: "mappin.circle.fill")
                .font(.title3)
            VStack(alignment: .leading, spacing: 1) {
                // `String(localized:)` rather than a literal in `Text`:
                // a ternary yields a String, which picks SwiftUI's
                // NON-localizing overload — the word would have shipped in
                // English in all eight languages.
                Text(attachment.name?.isEmpty == false ? attachment.name! : String(localized: "Location"))
                    .font(.callout)
                    .lineLimit(1)
                if !coordinateLine.isEmpty {
                    Text(verbatim: coordinateLine)
                        .font(.caption2)
                        .opacity(0.75)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
    }

    /// Coordinates, and the accuracy when the sender's device knew one.
    /// `Text(verbatim:)` because this is numbers and a unit, with nothing
    /// to translate — and because a localized decimal separator here would
    /// read as a different place.
    private var coordinateLine: String {
        guard let point = attachment.coordinate else { return "" }
        let latitude = String(format: "%.5f", locale: Locale(identifier: "en_US_POSIX"), point.latitude)
        let longitude = String(format: "%.5f", locale: Locale(identifier: "en_US_POSIX"), point.longitude)
        guard let accuracy = attachment.accuracyM else { return "\(latitude), \(longitude)" }
        return "\(latitude), \(longitude) · ±\(accuracy) m"
    }

    private var accessibilityText: String {
        let name = attachment.name?.isEmpty == false ? attachment.name! : String(localized: "Location")
        return "\(name). \(coordinateLine)"
    }

    /// Hand off to the system map app.
    ///
    /// `MKMapItem.openInMaps` rather than a `maps://` URL: it is the
    /// supported route on both platforms and it carries the label, so the
    /// pin arrives named rather than as a bare dot.
    private func open(_ coordinate: CLLocationCoordinate2D) {
        // `MKMapItem(location:address:)` on the newest SDKs, with the
        // placemark initialiser as the fallback — this app targets iOS 17
        // and macOS 14, where the newer one does not exist.
        let item: MKMapItem
        if #available(iOS 26.0, macOS 26.0, *) {
            item = MKMapItem(
                location: CLLocation(latitude: coordinate.latitude, longitude: coordinate.longitude),
                address: nil)
        } else {
            item = MKMapItem(placemark: MKPlacemark(coordinate: coordinate))
        }
        item.name = attachment.name?.isEmpty == false ? attachment.name : String(localized: "Location")
        item.openInMaps()
    }
}
