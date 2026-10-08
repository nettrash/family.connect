//
//  ComposerDrafts.swift
//  FamilyConnect
//
//  A parking space for a composer's unsent text across a view identity
//  change. The conversation views carry their draft in @State, and two
//  things legitimately destroy that state with the reader mid-sentence: a
//  tapped notification replacing the routed chat in place (the phone's
//  `.id(chatID)`, whose stale-state alternative was worse — the draft bled
//  into the WRONG chat), and the Mac's sidebar doing the same through
//  `.id(selectedChatID)` since the day it was written.
//
//  In-memory on purpose: a draft is a fragment of an unsent thought, not
//  data — it survives navigation, not the process. Only the TEXT parks
//  here; staged attachments and an edit session are deliberately let go (a
//  photo can be picked again, and an edit's banner names a message the
//  reader is no longer looking at).
//
//  RECORDINGS ARE NOT LET GO any more (#79, S2.8). A recording running when
//  the view dies, and a voice note still in review, are kept on disk as the
//  chat's "Voice message not sent" row — ParkedRecordings — because a
//  recording cannot be made again. A note in review takes the words in the
//  field along as its caption, so a composer that parked one stashes an
//  empty draft here.
//
import Foundation

@MainActor
enum ComposerDrafts {
    private static var byChat: [Int64: String] = [:]

    /// Park a draft when its view is dying. Empty text clears instead —
    /// coming BACK to a chat whose draft was sent must not resurrect it.
    static func stash(_ draft: String, for chatID: Int64) {
        let trimmed = draft
        if trimmed.isEmpty {
            byChat[chatID] = nil
        } else {
            byChat[chatID] = trimmed
        }
    }

    /// The parked draft, handed over exactly once.
    static func take(for chatID: Int64) -> String? {
        defer { byChat[chatID] = nil }
        return byChat[chatID]
    }
}
