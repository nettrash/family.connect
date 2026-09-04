//
//  ShareImport.swift
//  FamilyConnect
//
//  The app's half of the share extension — what is left of it once the
//  hand-off CONTRACT moved out.
//
//  The scheme, the host, the App Group, the inbox folder, the ten-item
//  cap and the URL parser now live in ShareHandoff (FamilyConnectShared),
//  compiled into the app AND the appex so the two cannot drift; a drift
//  there is silent, and a silently evaporating share is the worst kind of
//  bug in a family chat. What stays here is the one rule that is app
//  policy rather than contract: which chats a shared file may land in.
//
//  NOTHING AUTO-SENDS. The staged files land in a chat's composer as
//  staged attachments, and the user presses Send there — sharing into a
//  family chat is choosing to say something, not having said it.
//

import Foundation

nonisolated enum ShareImport {

    /// May a shared file land in this chat? Family and direct chats take
    /// attachments; the assistant's chat ("ai") does not — it is a text
    /// conversation with a model, and the server would refuse the send.
    ///
    /// App-side only: the extension never sees a chat list. It asks for a
    /// chat, not for a kind, which is why it stayed behind when the rest
    /// of the hand-off moved to ShareHandoff.
    static func isEligible(chatKind: String) -> Bool {
        chatKind != "ai"
    }
}
