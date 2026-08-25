//
//  ComposerPriming.swift
//  FamilyConnect
//
//  Why the composer is primed, and what the reader could see the moment it
//  happened.
//
//  There are two ways to borrow the composer for a message that is already
//  in the thread — answering it and rewriting it — and both do the same
//  thing to the layout: a banner appears, the draft is prefilled, and the
//  composer GROWS into the thread's viewport. Anything that grows the
//  composer takes the bottom sentinel out of the viewport, so the re-pin
//  that follows can no longer ask "were they at the newest message a moment
//  ago?" — `isPinnedToBottom` has already flipped to false, for the wrong
//  reason. The answer therefore has to be captured BEFORE the banner
//  appears, and it is captured here.
//
//  It is a value, and it takes the answer as an argument, because the flag
//  it replaces was set at only ONE of the two doors: replying captured it,
//  editing did not, so rewriting a message from last week's history threw
//  the reader down to the newest message while they were typing into it.
//  A door that cannot be opened without answering the question is the only
//  version of this that a third kind of banner cannot get wrong.
//
//  The phone spells the same rule as a Bool set in both of its own
//  `beginReply`/`beginEdit` (ConversationView) — same rule, same moment.
//

import Foundation

/// The state of a composer that has been primed from an existing message.
struct ComposerPriming: Equatable {

    /// The two ways a message can borrow the composer. Mutually exclusive
    /// by construction, which is also the rule both views state: you are
    /// answering a message or rewriting one, never both.
    enum Kind: Equatable {
        case reply
        case edit
    }

    let kind: Kind

    /// Was the newest message on screen when the banner appeared? Read from
    /// the bottom sentinel BEFORE the composer grew, which is the only
    /// moment it is still true.
    let startedAtNewest: Bool

    /// Private so the two factories below are the only doors, and both of
    /// them take the answer: there is no way to prime a composer without
    /// saying what the reader could see.
    private init(kind: Kind, startedAtNewest: Bool) {
        self.kind = kind
        self.startedAtNewest = startedAtNewest
    }

    static func reply(isPinnedToBottom: Bool) -> ComposerPriming {
        ComposerPriming(kind: .reply, startedAtNewest: isPinnedToBottom)
    }

    static func edit(isPinnedToBottom: Bool) -> ComposerPriming {
        ComposerPriming(kind: .edit, startedAtNewest: isPinnedToBottom)
    }

    /// Must a composer-height change be left alone rather than re-pinning
    /// the thread to its newest message?
    ///
    /// Yes exactly when the composer was primed from HISTORY: the reader is
    /// looking at the message they are answering or rewriting, and moving
    /// them away from it is the whole bug. A composer primed at the bottom
    /// still re-pins, or the banner's growth parks the newest message
    /// behind it.
    var suppressesRePin: Bool { !startedAtNewest }
}
