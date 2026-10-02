//
//  PictureRequestHint.swift
//  FamilyConnect
//
//  The one line a member reads WHILE asking for a picture: that real names
//  and brands are often refused (docs/protocol.md, "Pictures" — "A refused
//  description is reworded once").
//
//  The picture provider's own filter refuses far more than a text model's
//  does — almost any real person, public figure, brand or trademarked
//  character — and nothing tells a family which word was the problem. The
//  server now rewords a refused description once and tries again, but a
//  description written in general words in the first place is drawn
//  exactly as asked, where the rewrite is drawn as the model reworded it.
//  So the advice is said where the description is being written, the
//  doctrine the picture notices follow: at the moment it matters, not on a
//  settings screen somebody read once.
//
//  Two places, one rule each, extracted from the four views (the phone's
//  and the Mac's composer, the phone's note sheet and the Mac's note menu)
//  so iOS and macOS cannot drift on it and a test can pin every row with
//  no view on screen:
//
//  - a composer, while the draft is being written as a `/draw` request on
//    a server that can draw, and only where such a draft reaches the
//    assistant at all;
//  - beside the event-backdrop control, exactly where that control is
//    offered — the backdrop is drawn from the note's title, which is a
//    picture description by another name.
//
//  Android counterpart: the same sentence beside its composer and its
//  backdrop control.
//

import Foundation

nonisolated enum PictureRequestHint {
    /// The sentence itself — one key, shared by every place that says it.
    static var text: String {
        String(localized: "Describe people and things in general words — real names and brands are often refused.")
    }

    /// Does the composer show the hint under this draft?
    ///
    /// All of these, and each one is a reason the sentence would otherwise
    /// be advice about something that is not happening:
    ///
    /// - the server can draw, with the token this build speaks
    ///   (`AssistantSurfaces.offersPictureRequests`) — on a server without
    ///   an images deployment `/draw` is answered in words, and there is no
    ///   picture filter to warn about;
    /// - the composer is not borrowed for an edit — rewriting an old
    ///   message calls no model;
    /// - the draft is being written as a picture request: `/draw` and
    ///   whitespace, after leading whitespace and at most one leading `@ai`
    ///   (`AssistantMention.startsPictureRequest`) — from the moment the
    ///   token is typed, before any description follows it;
    /// - the draft reaches the assistant in this chat at all
    ///   (`AssistantConsent.reachesTheModel`): always in the assistant's
    ///   own chat, and in the family chat only with the `@ai` that makes
    ///   `@ai /draw` a request there. A bare `/draw` in the family chat, or
    ///   anything in a direct chat, is an ordinary message.
    static func showsInComposer(
        chatKind: String?,
        draft: String,
        isEditing: Bool,
        offersPictures: Bool
    ) -> Bool {
        guard offersPictures, !isEditing else { return false }
        guard AssistantMention.startsPictureRequest(draft) else { return false }
        return AssistantConsent.reachesTheModel(chatKind: chatKind, body: draft)
    }

    /// Is the event-backdrop control offered for this note — and so the
    /// hint beside it?
    ///
    /// One answer for both, so the sentence can never be left beside a
    /// control that is not there. An EVENT only (the other kinds have
    /// nowhere to put a backdrop), one the server already holds (a note
    /// being written has no id to draw for), its AUTHOR only (the backdrop
    /// is part of what the note looks like, which is the author's), and a
    /// server with an images deployment (`assistant.images`; without one
    /// the answer would be `pictures_unavailable`) — docs/protocol.md,
    /// "Board".
    ///
    /// The views ask `BackdropDoor`, which starts from this and adds the
    /// assistant's consent: a server that names no processor offers no
    /// backdrop at all, and one the author has not agreed to asks first.
    static func offersBackdrop(
        isEvent: Bool,
        isSaved: Bool,
        isAuthor: Bool,
        serverCanDraw: Bool
    ) -> Bool {
        isEvent && isSaved && isAuthor && serverCanDraw
    }
}
