//
//  MacSheetSizingTests.swift
//  FamilyConnectTests
//
//  What made five Mac sheets spill past their own windows, pinned so it
//  cannot come back quietly.
//
//  THE DEFECT. `ChangePasswordView` and four siblings are shared
//  iOS/macOS `NavigationStack { Form { … } }` screens, presented on the Mac
//  as sheets with a width and nothing else. macOS defaults a `Form` to the
//  COLUMNS style, and a columns form does not wrap to the width it is
//  offered — it reports a width LARGER than the proposal and expects the
//  window to grow. A `.frame(width:)` then centres that oversized content
//  inside the sheet, so it spills past BOTH edges: "input fields and
//  comments crossing the border of the window" (issue #67).
//
//  WHY THE FIX IS THE STYLE AND NOT THE NUMBER. This is the part worth a
//  test. Widening the frame does not help, because a columns form GROWS
//  with the offer — measured on the real password sheet, a 420pt proposal
//  returned 435pt and a 460pt proposal returned 528pt. There is no width at
//  which it settles. `.grouped` accepts the width it is offered and wraps,
//  which is what `macSheetForm()` applies.
//
//  WHAT IS ASSERTED, and why it needs no pinned font metrics: a probe form
//  is measured TWICE at the same proposal, once through `macSheetForm()`
//  and once without, and the two are compared AGAINST EACH OTHER. Grouped
//  must fit the offer; the default must not. That relationship survives a
//  new SDK and a different system font, which a hard-coded point count
//  would not — and if a future SDK ever made the columns style wrap on its
//  own, the second expectation fails loudly and says so, rather than
//  leaving a stale explanation in five files.
//
//  AND THE CEILINGS, which are what a wrong constant actually costs. A
//  macOS sheet cannot be resized by the person using it, so a sheet larger
//  than the window it hangs from simply overhangs it, for ever, in that
//  language. The check includes `MacSheetSize.actionBar`, because the
//  toolbar's Cancel/confirm buttons are laid out BELOW the frame a sheet is
//  given — that is how a 500pt constant first shipped a 565pt Delete
//  Account sheet into a 530pt Settings window — and it measures against
//  each parent's MINIMUM size rather than the size it opens at, because
//  Settings is resizable down to 420pt and the sheet is not.
//
//  WHAT THIS FILE STILL CANNOT ASSERT. That each height is TALL ENOUGH for
//  its own content in all nine languages. A sized sheet reports its frame
//  back through `sizeThatFits`, so the content behind it cannot be measured
//  through the view as shipped. Those numbers were chosen against measured
//  grouped intrinsic widths (the table in `MacSheetSize`) and still want one
//  look on a signed local build — which is also the only way to see these
//  dialogs at all, since an unsigned build cannot write the keychain
//  (OSStatus -34018).
//

#if os(macOS)

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
struct MacSheetSizingTests {

    /// The narrowest sheet in `MacSheetSize`, and so the tightest offer any
    /// of these forms is made.
    private static let proposal: CGFloat = 440

    /// A form shaped like the ones that broke: a labelled field and a
    /// footer long enough to need wrapping. The footer is the load-bearing
    /// part — a columns form lays its labels and fields out side by side
    /// and refuses to wrap the prose under them.
    private struct ProbeForm: View {
        let grouped: Bool

        var body: some View {
            Form {
                Section {
                    SecureField("Current Password", text: .constant(""))
                } footer: {
                    Text("Your other devices will be signed out. This one stays signed in.")
                }
            }
            .applyGrouped(grouped)
        }
    }

    private func width(grouped: Bool) -> CGFloat {
        let host = NSHostingController(rootView: ProbeForm(grouped: grouped))
        return host.sizeThatFits(
            in: CGSize(width: Self.proposal, height: CGFloat.greatestFiniteMagnitude)).width
    }

    @Test("a grouped form accepts the width it is offered, and the default does not")
    func groupedFitsTheOfferAndColumnsDoesNot() {
        let grouped = width(grouped: true)
        let columns = width(grouped: false)

        #expect(
            grouped <= Self.proposal,
            """
            macSheetForm() reported \(Int(grouped))pt for a \(Int(Self.proposal))pt \
            offer, so the grouped style is no longer wrapping and every shared \
            Mac sheet is spilling past its window again — this is issue #67.
            """)
        #expect(
            columns > Self.proposal,
            """
            the DEFAULT form style reported \(Int(columns))pt for a \
            \(Int(Self.proposal))pt offer, i.e. it now fits. That is good news, \
            but the comments in PlatformStyle.swift, PasswordView.swift and this \
            file all explain the fix by the opposite behaviour — rewrite them \
            before deleting this expectation.
            """)
        #expect(
            columns > grouped,
            "the two styles measured the same, so the probe is not exercising the difference")
    }

    /// The five sheets, each with the parent it is presented from.
    private static let sheets: [(String, CGSize, CGSize)] = [
        ("changePassword", MacSheetSize.changePassword, MacSheetSize.settingsWindow),
        ("myBirthday", MacSheetSize.myBirthday, MacSheetSize.settingsWindow),
        ("deleteAccount", MacSheetSize.deleteAccount, MacSheetSize.settingsWindow),
        ("resetPassword", MacSheetSize.resetPassword, MacSheetSize.rosterSheet),
        ("memberBirthday", MacSheetSize.memberBirthday, MacSheetSize.rosterSheet),
    ]

    @Test("every shared sheet fits inside the window it is presented from, at that window's minimum")
    func sheetsFitTheirParents() {
        for (name, size, parent) in Self.sheets {
            #expect(
                size.width <= parent.width,
                """
                MacSheetSize.\(name) is \(Int(size.width))pt wide inside a \
                \(Int(parent.width))pt window — a sheet wider than its own parent, \
                which is the shape StatisticsView had to undo.
                """)
            #expect(
                size.height <= parent.height,
                """
                MacSheetSize.\(name) is \(Int(size.height))pt tall inside a window that \
                can be \(Int(parent.height))pt — it would overhang, and a macOS sheet \
                cannot be resized out of it. Remember these constants are the RENDERED \
                sheet: macSheetFrame subtracts \(Int(MacSheetSize.actionBar))pt of action \
                bar before framing, so shrinking this number shrinks the form, not the \
                chrome.
                """)
        }
    }

    /// The action bar is what `macSheetFrame` subtracts, so a form has to be
    /// left with a usable amount after it. This catches the mirror mistake of
    /// the one that shipped: a height so small that the frame goes to nothing.
    @Test("every sheet leaves a usable form area once the action bar is taken out")
    func formAreaSurvivesTheActionBar() {
        for (name, size, _) in Self.sheets {
            let form = size.height - MacSheetSize.actionBar
            #expect(
                form >= 300,
                """
                MacSheetSize.\(name) leaves only \(Int(form))pt of form once the \
                \(Int(MacSheetSize.actionBar))pt action bar is subtracted — the password \
                form alone measures 249pt of content (265 in French) before its error row.
                """)
        }
    }
}

extension View {
    /// `macSheetForm()` or nothing, chosen at runtime so one probe can be
    /// measured both ways.
    @ViewBuilder
    fileprivate func applyGrouped(_ grouped: Bool) -> some View {
        if grouped { macSheetForm() } else { self }
    }
}

#endif
