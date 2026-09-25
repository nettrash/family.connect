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
    ///
    /// The backdrop ignores the KEYBOARD's safe area on purpose. The
    /// outer frame shrinks when a keyboard (or its shortcut bar, with a
    /// hardware keyboard) rises, and a background that stopped with it
    /// left the window's white showing either side of the column below
    /// the fields — measured on a 13-inch iPad with a keyboard attached.
    @ViewBuilder
    func setupColumn() -> some View {
        #if os(iOS)
        frame(maxWidth: 460)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { Color(.systemGroupedBackground).ignoresSafeArea() }
        #else
        frame(maxWidth: 460)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.appBackground)
        #endif
    }
}

// MARK: - Shared sheets on the Mac

extension View {
    /// The form style a shared `Form` needs when it is presented as a macOS
    /// sheet. On iOS this is the identity.
    ///
    /// macOS defaults a `Form` to the **columns** style, and a columns form
    /// does not wrap to the width it is offered — it reports a width LARGER
    /// than the proposal and expects the window to grow. Measured on this
    /// app's own password sheet at a 420pt proposal: 435pt in English, 481pt
    /// in German, and at a 460pt proposal it grew to 528pt. A `.frame(width:)`
    /// then centres that oversized content inside the sheet, so it spills
    /// past BOTH edges of the window — which is what "input fields and
    /// comments crossing the border of the window" looks like from outside.
    ///
    /// `.grouped` accepts the width it is offered and wraps instead. **The
    /// fix is the style, not the number** — no width alone repairs a columns
    /// form, because the content grows with the offer.
    @ViewBuilder
    func macSheetForm() -> some View {
        #if os(macOS)
        formStyle(.grouped)
        #else
        self
        #endif
    }

    /// The size a shared `NavigationStack { Form { … } }` needs to be a
    /// well-behaved macOS sheet. On iOS this is the identity.
    ///
    /// **A macOS sheet cannot be resized by the person using it**, so it is
    /// sized here or it is wrong for everybody, in every language. Size for
    /// the TALLEST state the sheet can reach, not the state it opens in:
    /// every one of these forms grows an error row only after a failed save,
    /// which is the likeliest moment of all in a password dialog.
    ///
    /// **The size is the sheet, and the frame is smaller than it.** SwiftUI
    /// lays a `NavigationStack`'s `.cancellationAction` and
    /// `.confirmationAction` buttons in a bar BELOW the frame it is given, so
    /// a `.frame(height: 500)` renders a 565pt sheet. That is measured, not
    /// assumed — a 440x380 probe reports `FocusGuideView frame={{0,380},{440,65}}`
    /// under a 440x380 container — and it is exactly how a 500pt constant put
    /// a 565pt Delete Account sheet inside a 530pt Settings window. So
    /// [`MacSheetSize`] holds what the SHEET measures and the action bar is
    /// subtracted here, once, where the mistake cannot be made again.
    ///
    /// **The title is NOT set here.** A `NavigationStack` in a macOS sheet
    /// does render its own bar and its own `.navigationTitle` — measured:
    /// removing `.navigationTitle` from the probe removes a 47.5pt
    /// `_NSGraphicsView` from the tree and the title with it. An earlier
    /// version of this helper added a `safeAreaInset` headline in the belief
    /// that it did not, and drew every title twice. `ReportSheet` is the
    /// in-tree shape that gets this right and always did.
    ///
    /// Apply this OUTSIDE the `NavigationStack`, and `macSheetForm()` on the
    /// `Form` inside it. Nothing may re-frame the result at the call site: an
    /// outer `.frame(width:)` clamps this one, and the overflow survives a fix
    /// that looks applied.
    @ViewBuilder
    func macSheetFrame(_ size: CGSize) -> some View {
        #if os(macOS)
        frame(width: size.width, height: size.height - MacSheetSize.actionBar)
        #else
        self
        #endif
    }
}

/// Every shared sheet's size, in one table — the size of the SHEET, as it is
/// drawn.
///
/// They are gathered here rather than left as five pairs of numbers in five
/// files because the constraint that decides them is not local: a macOS sheet
/// cannot be resized by the person using it, and it must also fit inside the
/// window it hangs from. `StatisticsView` records what happens otherwise — a
/// 520x560 sheet inside the 460-wide Settings window was wider and taller than
/// its own window.
///
/// **These are rendered sizes, not frames.** `macSheetFrame` subtracts
/// ``actionBar`` before framing, because a `NavigationStack`'s Cancel/confirm
/// buttons are laid out BELOW the frame it is given. Getting that backwards is
/// how a 500pt constant first shipped a 565pt sheet into a 530pt window.
///
/// The two parents, and therefore the two ceilings — measured against each
/// window's MINIMUM rather than the size it opens at, because a person may drag
/// Settings down to its minimum and a sheet cannot shrink with it:
///
/// - **Settings** is pinned `minWidth 460 / minHeight 420` and opens at 460x530
///   (`MacSettingsView`, `FamilyConnectApp`), so its sheets fit inside 460x420.
/// - **MacFamilyView** is itself a 520x520 sheet on the main window and does
///   not resize, so the two sheets the roster opens have more room.
///
/// Widths are chosen against MEASURED grouped intrinsic widths across the nine
/// shipped languages: zh-Hans 279, sr-Latn 351, en 389, sr 399, fr 405, ja 409,
/// de 425, es 450, ru 476. Russian and Spanish wrap a footer line at these
/// numbers, which costs a line of height and nothing else; widening past the
/// parent window to avoid that would cost the whole dialog.
///
/// Heights are for the TALLEST state each sheet can reach, not the one it opens
/// in — every one of these forms grows an error row only after a failed save,
/// and that is the likeliest moment of all in a password dialog. Where a form
/// still will not fit, it SCROLLS, which is the right failure: a sheet that
/// overhangs its parent window has no graceful one.
enum MacSheetSize {
    /// The bar `NavigationStack` lays out below a sized sheet's frame for the
    /// toolbar's `.cancellationAction` and `.confirmationAction` buttons.
    ///
    /// Measured, not assumed: a 440x380 probe of this exact shape reports
    /// `FocusGuideView frame={{0,380},{440,65}}` beneath a 440x380 container.
    /// Subtracted in one place so every number below can mean the thing a
    /// person actually sees.
    static let actionBar: CGFloat = 65

    /// Settings ▸ Change Password.
    static let changePassword = CGSize(width: 440, height: 415)
    /// Settings ▸ Birthday.
    static let myBirthday = CGSize(width: 440, height: 415)
    /// Settings ▸ Delete Account — four paragraphs of consequences before the
    /// password field, so this is the one that genuinely scrolls. It is held to
    /// the same ceiling as its neighbours rather than given the room it would
    /// like: scrolling costs less than overhanging.
    static let deleteAccount = CGSize(width: 440, height: 415)
    /// The roster's Reset Password — its footer is a whole paragraph carrying
    /// a member's display name.
    static let resetPassword = CGSize(width: 460, height: 460)
    /// The roster's Birthday — its section header carries a display name.
    static let memberBirthday = CGSize(width: 460, height: 440)

    /// What each sheet's parent offers at its SMALLEST, which is the size the
    /// sheet actually has to fit: Settings is resizable down to its minimum,
    /// and the roster sheet is not resizable at all.
    static let settingsWindow = CGSize(width: 460, height: 420)
    static let rosterSheet = CGSize(width: 520, height: 520)
}
