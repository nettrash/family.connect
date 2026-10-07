//
//  MacHostWindow.swift
//  FamilyConnect
//
//  Which NSWindow a SwiftUI view is drawn in.
//
//  SwiftUI on the Mac says nothing about a window being minimised, and a
//  minimised window must stop a recording while one that merely lost focus
//  must not (#79, S4). AppKit says it — `NSWindow.willMiniaturizeNotification`
//  — about SOME window, so telling "this conversation's window" from "some
//  other window" needs the window itself. This is the smallest way to get
//  it: an empty NSView that reports the window it lands in.
//

#if os(macOS)

import AppKit
import SwiftUI

/// The window, held WEAKLY: a view's state that held its own window
/// strongly would be a cycle through the window's content view.
final class MacWindowBox {
    weak var window: NSWindow?
}

/// Reports the window it is drawn in, every time it moves to one.
struct MacHostWindowReader: NSViewRepresentable {
    let box: MacWindowBox

    func makeNSView(context: Context) -> NSView {
        Reporter(box: box)
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class Reporter: NSView {
        let box: MacWindowBox

        init(box: MacWindowBox) {
            self.box = box
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) is not used")
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            box.window = window
        }
    }
}

#endif
