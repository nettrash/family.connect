#if os(macOS)
import AppKit
import Carbon.HIToolbox
import SwiftUI

/// AppKit's field editor consumes arrow keys before SwiftUI sees them.
/// Listen in the picker window so Down can leave search and the grid can
/// handle arrows consistently with either macOS keyboard-navigation setting.
struct MacReactionPickerArrowKeys: NSViewRepresentable {
    let handle: (KeyEquivalent) -> Bool

    func makeNSView(context: Context) -> Observer { Observer() }

    func updateNSView(_ view: Observer, context: Context) {
        view.handle = handle
    }

    static func dismantleNSView(_ view: Observer, coordinator: ()) {
        view.stopObserving()
    }

    final class Observer: NSView {
        var handle: (KeyEquivalent) -> Bool = { _ in false }
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, let window, (event.window ?? NSApp.keyWindow) === window,
                      event.modifierFlags.intersection([.shift, .control, .option, .command]).isEmpty
                else { return event }

                let key: KeyEquivalent
                switch Int(event.keyCode) {
                case kVK_LeftArrow: key = .leftArrow
                case kVK_RightArrow: key = .rightArrow
                case kVK_UpArrow: key = .upArrow
                case kVK_DownArrow: key = .downArrow
                default: return event
                }
                return handle(key) ? nil : event
            }
        }

        func stopObserving() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}
#endif
