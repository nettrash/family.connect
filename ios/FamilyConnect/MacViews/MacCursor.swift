//
//  MacCursor.swift
//  FamilyConnect
//
//  "This is draggable / clickable", said the only way a Mac says it.
//
//  NSCursor.push()/pop() is a STACK, and the obvious spelling —
//  `.onHover { $0 ? cursor.push() : NSCursor.pop() }` — leaks a push
//  whenever the view goes away while the pointer is still over it. In a
//  scrolling thread that is not a corner case: hover a photo, scroll it out
//  from under the pointer, and the exit callback may never come. A few of
//  those and the whole window is stuck showing a pointing hand.
//
//  So the pop is owned by this modifier, which knows whether it pushed, and
//  pops on disappear too.
//
//  `.pointerStyle` (macOS 15) would replace all of this; the deployment
//  target is 14.
//

#if os(macOS)

import AppKit
import SwiftUI

private struct HoverCursor: ViewModifier {
    let cursor: NSCursor

    @State private var pushed = false

    func body(content: Content) -> some View {
        content
            .onHover { inside in
                if inside {
                    guard !pushed else { return }
                    pushed = true
                    cursor.push()
                } else {
                    pop()
                }
            }
            .onDisappear(perform: pop)
    }

    private func pop() {
        guard pushed else { return }
        pushed = false
        NSCursor.pop()
    }
}

extension View {
    /// Show `cursor` while the pointer is over this view — balanced even if
    /// the view disappears mid-hover.
    func hoverCursor(_ cursor: NSCursor) -> some View {
        modifier(HoverCursor(cursor: cursor))
    }
}

#endif
