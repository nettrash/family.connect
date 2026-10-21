//
//  HardwareReturn.swift
//  FamilyConnect
//
//  The two facts a hardware Return needs on iPhone and iPad that SwiftUI's
//  multi-line field does not hand over (#106, 2.3): which modifiers were
//  held, and how a line break is added to a draft the field just submitted.
//
//  WHAT THE FIELD DOES WITH IT. On iOS 27 a hardware Return in a
//  `TextField(axis: .vertical)` arrives as `onSubmit` — Shift+Return too —
//  and `onKeyPress(.return)` never fires; a SwiftUI
//  `keyboardShortcut(.return, modifiers: .shift)` loses to the field as
//  well (all measured with XCUITest's `typeKey("\n")`). So:
//
//  - ⌥↩ and ⌃↩ are a new line already: the field inserts one itself.
//  - Shift+Return is marked by a UIKit key command that asks for priority
//    over the field's own Return (`ReturnKeyCommands`), installed only
//    while the composer's field has the focus, and answered with a new
//    line whether or not a submit follows it.
//  - Return alone is the field's submit, and sends.
//  - A submit carries no event, so `held()` also reads the modifiers off
//    the keyboard's live state (GameController) as a second net: on a
//    device it sees a Shift the key command somehow missed; where it can
//    see nothing (the simulator under XCUITest), nothing is held, and
//    Return sends — the answer a person who pressed it bare wants.
//

#if os(iOS)

import GameController
import SwiftUI
import UIKit

enum HardwareReturn {

    struct Held: Equatable {
        var shift = false
        var option = false
        var command = false
        var control = false
    }

    /// The modifiers held on the attached keyboard right now.
    @MainActor
    static func held() -> Held {
        guard let input = GCKeyboard.coalesced?.keyboardInput else { return Held() }
        func down(_ codes: GCKeyCode...) -> Bool {
            codes.contains { input.button(forKeyCode: $0)?.isPressed == true }
        }
        return Held(
            shift: down(.leftShift, .rightShift),
            option: down(.leftAlt, .rightAlt),
            command: down(.leftGUI, .rightGUI),
            control: down(.leftControl, .rightControl))
    }

    /// The draft with a line break at its end — or nil when the protocol's
    /// 4000-character ceiling leaves no room for one (`ComposerText`).
    nonisolated static func appendingLine(to draft: String) -> String? {
        guard case .appended(let extended) = ComposerText.appending("\n", to: draft) else { return nil }
        return extended
    }
}

/// Shift+Return as a new line (see the header). A zero-sized view in the
/// field's background: while `active` it adds a Shift+Return key command to
/// the view controller that hosts it — an ancestor of the field, so on the
/// responder chain the press walks — and takes it away the moment the field
/// lets the focus go, so no other text field ever meets it. The on-screen
/// keyboard sends no key commands at all, so it is untouched. ⌥↩ and ⌃↩ need
/// nothing: the field already makes them a new line (measured).
///
/// The command asks for priority over the field's own Return, but whether
/// UIKit lets it swallow the press is not this side's to assert, so the
/// answer is written to hold either way: the command only marks the press,
/// and the line break is added ONCE — by the field's submit if one follows
/// (`takeShiftReturn`, asked first thing in the submit), otherwise a beat
/// later by the command itself.
struct ReturnKeyCommands: UIViewRepresentable {
    /// The composer's field has the focus.
    let active: Bool
    let onNewLine: () -> Void

    func makeUIView(context: Context) -> Host { Host() }

    func updateUIView(_ host: Host, context: Context) {
        host.onNewLine = onNewLine
        host.active = active
    }

    static func dismantleUIView(_ host: Host, coordinator: ()) {
        host.active = false
    }

    /// The field whose key command is live — one at a time, the focused one.
    static weak var current: Host?

    /// Whether the submit being handled is a Shift+Return the key command
    /// saw — and if so, it is the submit's to answer, with a new line.
    static func takeShiftReturn() -> Bool {
        guard let current, current.pending else { return false }
        current.pending = false
        return true
    }

    final class Host: UIView {
        var onNewLine: () -> Void = {}
        var active = false {
            didSet { if active != oldValue { install() } }
        }
        /// A Shift+Return was pressed and no line break answered it yet.
        fileprivate var pending = false
        private var lastPress: CFTimeInterval = 0
        private weak var installedOn: UIViewController?

        private lazy var command: UIKeyCommand = {
            let command = UIKeyCommand(
                input: "\r", modifierFlags: .shift,
                action: #selector(UIViewController.familyConnectShiftReturn(_:)))
            // Without it the field's own Return wins outright and submits —
            // which is what this exists to stop.
            command.wantsPriorityOverSystemBehavior = true
            return command
        }()

        override init(frame: CGRect) {
            super.init(frame: frame)
            isUserInteractionEnabled = false
            isAccessibilityElement = false
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            install()
        }

        fileprivate func pressed() {
            let now = CACurrentMediaTime()
            // One press, one line, however many times UIKit reports it.
            guard now - lastPress > 0.08 else { return }
            lastPress = now
            pending = true
            // Long enough for a submit the press may still cause to come
            // first and claim it; short enough to read as the key's own.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
                guard let self, self.pending else { return }
                self.pending = false
                self.onNewLine()
            }
        }

        private func install() {
            let target = active && window != nil ? hostingController : nil
            if target !== installedOn {
                installedOn?.removeKeyCommand(command)
                target?.addKeyCommand(command)
                installedOn = target
            }
            if target != nil {
                ReturnKeyCommands.current = self
            } else if ReturnKeyCommands.current === self {
                ReturnKeyCommands.current = nil
                pending = false
            }
        }

        /// The nearest view controller above this view: the one hosting the
        /// SwiftUI tree the field is drawn in.
        private var hostingController: UIViewController? {
            var responder: UIResponder? = self
            while let next = responder?.next {
                if let controller = next as? UIViewController { return controller }
                responder = next
            }
            return nil
        }
    }
}

extension UIViewController {
    /// The action of `ReturnKeyCommands`' key command. On every view
    /// controller so the responder chain finds it at whichever one hosts
    /// the field; it does nothing unless a composer's field is focused.
    @objc func familyConnectShiftReturn(_ sender: UIKeyCommand) {
        ReturnKeyCommands.current?.pressed()
    }
}

#endif
