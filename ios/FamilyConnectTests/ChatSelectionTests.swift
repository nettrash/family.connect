//
//  ChatSelectionTests.swift
//  FamilyConnectTests
//
//  The chat list has two shapes since issue #43 — a NavigationStack on
//  the phone, a NavigationSplitView on the iPad — over ONE piece of
//  state, `ChatListView.path`. That state is also the app's single
//  landing surface for the routes that are hardest to reproduce and
//  worst to break: a cold-start push tap, a call the system asks to be
//  placed from Siri or Recents, and a share handed off by the Share
//  Extension.
//
//  Not one of those routes was changed by the conversion — they still
//  open a chat by writing the path. So what has to hold is one sentence:
//  whatever they write, the split view must open. Each test below is one
//  of the eight writes, replayed on the projection the sidebar reads.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Chat selection ↔ navigation path")
struct ChatSelectionTests {

    @Test("nothing open is no selection, and vice versa")
    func empty() {
        #expect(ChatSelection.openChat(in: []) == nil)
        #expect(ChatSelection.path(opening: nil) == [])
    }

    /// `path = [chatID]` — a tapped push notification (PushRoute .chat),
    /// a system call request (`place(callTo:video:)`), and the share
    /// picker all write exactly this.
    @Test("a route that replaces the path opens that chat")
    func replacement() {
        #expect(ChatSelection.openChat(in: [42]) == 42)
        #expect(ChatSelection.path(opening: 42) == [42])
    }

    /// `path = []` — the board, join-requests and reports push routes all
    /// clear the path before raising their sheet. On a stack that pops to
    /// the list; in a split view it must DESELECT, or the sheet would come
    /// up over a thread that the route asked to leave.
    @Test("a route that clears the path closes the thread")
    func clearing() {
        #expect(ChatSelection.openChat(in: []) == nil)
    }

    /// `path.append(chatID)` — the New Chat sheet. On the stack the path
    /// is empty when that button can be pressed, so append == replace; in
    /// the sidebar a chat may already be open, and the freshly created one
    /// is what the person asked to see.
    @Test("New Chat's append opens the chat it just created")
    func append() {
        var path: [Int64] = []
        path.append(7)
        #expect(ChatSelection.openChat(in: path) == 7)

        var withOneOpen: [Int64] = [42]
        withOneOpen.append(7)
        #expect(ChatSelection.openChat(in: withOneOpen) == 7)
    }

    /// `path.removeAll { vanished.contains($0) }` — a direct chat whose
    /// peer deleted their account disappears under the reader
    /// (protocol.md, "Deleting an account"). Standing in it afterwards is
    /// a thread where every request answers 404, so the detail column has
    /// to fall back to the empty state.
    @Test("a chat that vanishes deselects itself")
    func vanished() {
        var path: [Int64] = [42]
        let vanished: Set<Int64> = [42]
        path.removeAll { vanished.contains($0) }
        #expect(ChatSelection.openChat(in: path) == nil)
    }

    /// Only what disappeared. A chat that vanishes while ANOTHER one is
    /// open must not close the one being read.
    @Test("a chat vanishing elsewhere leaves the open one alone")
    func vanishedElsewhere() {
        var path: [Int64] = [9]
        let vanished: Set<Int64> = [42]
        path.removeAll { vanished.contains($0) }
        #expect(ChatSelection.openChat(in: path) == 9)
    }

    /// The sidebar writing back: tapping a row hands the split view a new
    /// selection, and the path must end up holding exactly that chat —
    /// including when a New Chat append had left two entries behind.
    @Test("selecting from the sidebar leaves exactly that chat open")
    func selectionRoundTrip() {
        for chatID in [Int64(1), 42, .max] {
            let path = ChatSelection.path(opening: chatID)
            #expect(path == [chatID])
            #expect(ChatSelection.openChat(in: path) == chatID)
        }
        #expect(ChatSelection.path(opening: ChatSelection.openChat(in: [42, 7])) == [7])
    }
}
