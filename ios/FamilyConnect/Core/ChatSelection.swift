//
//  ChatSelection.swift
//  FamilyConnect
//
//  The one piece of arithmetic that lets the chat list have two shapes
//  (issue #43): a NavigationStack navigating a `path` on the phone, and a
//  NavigationSplitView driving a `selection` on the iPad, over ONE piece
//  of state.
//
//  It matters far more than three lines usually would. `ChatListView.path`
//  is the app's single landing surface for a cold-start push tap, for a
//  Siri or Recents call the system asks the app to place, and for a share
//  handed off from the Share Extension — the routes that are hardest to
//  reproduce and worst to break. Every one of them opens a chat by
//  WRITING THE PATH, and none of them was touched by the split view: they
//  still write the path, and this is what the split view reads it as. So
//  the invariant those routes actually depend on — "writing the path opens
//  that chat, clearing it closes the thread" — is testable here, on both
//  shapes at once, with no simulator and no live server.
//
//  Not shared with the Mac: MacChatView keeps a plain `selectedChatID`
//  because it never had a stack to be deep in.
//

import Foundation

enum ChatSelection {
    /// The chat a path has open: its LAST element.
    ///
    /// The last, not the first, because the New Chat sheet appends
    /// (`path.append(chatID)`) rather than replaces. On the stack that is
    /// the same thing — the button only exists on the list, so the path is
    /// empty when it is pressed — but in a sidebar a chat may already be
    /// open, and what the person just created is what they want to see.
    static func openChat(in path: [Int64]) -> Int64? {
        path.last
    }

    /// The path that has one chat open, or none.
    ///
    /// Collapses to a single element rather than appending: a split view
    /// has no stack to be deep in, so selecting is replacing. The routes
    /// that write the path directly already say `path = [chatID]` for
    /// exactly this reason.
    static func path(opening chatID: Int64?) -> [Int64] {
        guard let chatID else { return [] }
        return [chatID]
    }
}
