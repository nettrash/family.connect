#if os(macOS)
import AppKit
import SwiftUI

/// The AppKit field editor handles Down before SwiftUI's TextField sees it.
/// Intercept only an unmodified Down in the picker's own window, and let
/// the picker decline it while the search field is composing marked text.
struct MacReactionPickerSearchKeys: NSViewRepresentable {
    let moveToResults: () -> Bool

    func makeNSView(context: Context) -> Observer { Observer() }

    func updateNSView(_ view: Observer, context: Context) {
        view.moveToResults = moveToResults
    }

    static func dismantleNSView(_ view: Observer, coordinator: ()) {
        view.stopObserving()
    }

    final class Observer: NSView {
        var moveToResults: () -> Bool = { false }
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, let window, event.window === window,
                      event.specialKey == .downArrow,
                      event.modifierFlags.intersection([.shift, .control, .option, .command]).isEmpty,
                      moveToResults()
                else { return event }
                return nil
            }
        }

        func stopObserving() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}
#endif
