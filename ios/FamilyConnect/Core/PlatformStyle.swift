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
    /// No-op on iOS, where the window IS the column.
    @ViewBuilder
    func setupColumn() -> some View {
        #if os(iOS)
        self
        #else
        frame(maxWidth: 460)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.appBackground)
        #endif
    }
}
