//
//  PlatformStyle.swift
//  FamilyConnect
//
//  The handful of places where the same intent has two spellings.
//
//  Deliberately small. Anything genuinely different between the platforms
//  — navigation shape, the composer, tap-and-hold menus — belongs in views
//  written for that platform, not behind a shim. What is here is the
//  vocabulary a shared screen needs: system colours that exist on both
//  under different names, and one modifier that means something on iOS and
//  nothing on the Mac.
//

import SwiftUI

extension Color {
    /// The window/page background. `systemBackground` on iOS,
    /// `windowBackgroundColor` on the Mac — the same role, two names.
    static var appBackground: Color {
        #if os(iOS)
        Color(.systemBackground)
        #else
        Color(nsColor: .windowBackgroundColor)
        #endif
    }

    /// The recessed fill behind an input field or a chip.
    static var appSecondaryFill: Color {
        #if os(iOS)
        Color(.secondarySystemFill)
        #else
        Color(nsColor: .quaternaryLabelColor)
        #endif
    }

    /// A hairline between things — a card's border, a divider.
    static var appSeparator: Color {
        #if os(iOS)
        Color(.separator)
        #else
        Color(nsColor: .separatorColor)
        #endif
    }

    /// The grouped-list backdrop a settings screen sits on.
    static var appGroupedBackground: Color {
        #if os(iOS)
        Color(.systemGroupedBackground)
        #else
        Color(nsColor: .underPageBackgroundColor)
        #endif
    }
}

extension View {
    /// Text entry that must not be "helped": usernames, invite codes,
    /// server addresses. macOS has no autocapitalisation to switch off,
    /// so there the modifier is just autocorrection.
    @ViewBuilder
    func literalTextEntry(uppercased: Bool = false) -> some View {
        #if os(iOS)
        self
            .textInputAutocapitalization(uppercased ? .characters : .never)
            .autocorrectionDisabled()
        #else
        autocorrectionDisabled()
        #endif
    }

    /// `navigationBarTitleDisplayMode(.inline)`, which does not exist on
    /// macOS — a Mac window title has no large/inline distinction.
    @ViewBuilder
    func inlineNavigationTitle() -> some View {
        #if os(iOS)
        navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }

    /// Hold a setup screen to a readable column, centred in the window.
    ///
    /// The screens before the chat itself — server address, log in, join a
    /// family — are the phone's, and rightly so: they are four fields and a
    /// button, and writing them twice would buy nothing. But a phone form
    /// stretched across a thousand points of Mac window is the "iPad app on
    /// a Mac" look, a label at one edge and its field at the other. Every
    /// Mac setup sheet answers this the same way, so this does too.
    ///
    /// The same holds on a big iPad, which is why this is no longer a
    /// no-op there. On a phone the window really IS the column — 460pt is
    /// wider than every iPhone, so the clamp never binds and the phone
    /// renders exactly as it always has. On a 13-inch iPad it was binding
    /// on nothing: the server-address field ran 992pt in portrait and
    /// 1344pt in landscape, roughly four times the width of the one line
    /// of help text beneath it, with the button a second full-bleed pill
    /// under it.
    ///
    /// Applied here, OUTSIDE each screen's NavigationStack, rather than
    /// inside the six screens: measured, the large title then sits inside
    /// the centred column, and the bar is transparent at rest so no chrome
    /// floats. Clamping inside the Form instead strands the title at the
    /// far left of the window, hundreds of points from the form it names.
    @ViewBuilder
    func setupColumn() -> some View {
        #if os(iOS)
        frame(maxWidth: 460)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.systemGroupedBackground))
        #else
        frame(maxWidth: 460)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.appBackground)
        #endif
    }
}
