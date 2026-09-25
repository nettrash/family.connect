//
//  MentionSuggestions.swift
//  FamilyConnect
//
//  The roster offered while a member types `@` (docs/protocol.md,
//  "Mentioning a member"): a strip of names above the composer, narrowed as
//  the name is typed. A strip rather than a popover for the same reason the
//  assistant got a button: the composer is a plain TextField with no caret
//  API on the targets this app ships to, so the only edit it can make is to
//  the trailing token — which is exactly what a strip that rewrites
//  `@prefix` into `@Name ` does.
//
//  Shared by the phone, the Mac and the thread sheet. Android counterpart:
//  ChatScreen.MentionSuggestionsRow.
//

import SwiftUI

struct MentionSuggestions: View {
    let candidates: [MentionDTO]
    let onPick: (String) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(candidates, id: \.userID) { member in
                    Button {
                        onPick(member.name)
                    } label: {
                        Text(verbatim: "@" + member.name)
                            .font(.callout.weight(.medium))
                            .lineLimit(1)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Color.accentColor.opacity(0.12), in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Mention \(member.name)"))
                }
            }
            .padding(.horizontal, 12)
        }
        .padding(.top, 6)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}
