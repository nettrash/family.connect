//
//  MentionSuggestions.swift
//  FamilyConnect
//
//  The roster offered while a member types `@` (docs/protocol.md,
//  "Mentioning a member"): a strip of names above the composer, narrowed as
//  the name is typed. A strip rather than a popover for the reason the
//  composer always had: it is a plain TextField with no caret API on the
//  targets this app ships to, so the only edit it can make is to the
//  trailing token — which is exactly what a strip that rewrites `@prefix`
//  into `@Name ` does.
//
//  The family chat's assistant leads the strip since #106, when its button
//  left the composer row (MemberMentions.suggestions): drawn with the
//  sparkles the button wore, so the people who knew the button recognise
//  it, under its display name — and picking it writes `@ai `, the token, not
//  that name.
//
//  Shared by the phone, the Mac and the thread sheet. Android counterpart:
//  ChatScreen.MentionSuggestionsRow.
//

import SwiftUI

struct MentionSuggestions: View {
    let candidates: [MentionSuggestion]
    /// Called with what goes after the `@`: a member's name, or `ai`.
    let onPick: (String) -> Void

    /// The chat composers' strip: the assistant (where it is offered) and
    /// the members.
    init(suggestions: [MentionSuggestion], onPick: @escaping (String) -> Void) {
        self.candidates = suggestions
        self.onPick = onPick
    }

    /// Members only — the board's note editors and the thread sheet, where
    /// `@ai` is not answered from (a note asks nobody; a thread has no
    /// consent bar to stand between a typed `@ai` and the model).
    init(candidates: [MentionDTO], onPick: @escaping (String) -> Void) {
        self.candidates = candidates.map(MentionSuggestion.member)
        self.onPick = onPick
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(candidates, id: \.id) { candidate in
                    Button {
                        onPick(candidate.insertedName)
                    } label: {
                        chip(candidate)
                            .font(.callout.weight(.medium))
                            .lineLimit(1)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Color.accentColor.opacity(0.12), in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Mention \(candidate.shownName)"))
                }
            }
            .padding(.horizontal, 12)
        }
        .padding(.top, 6)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    @ViewBuilder
    private func chip(_ candidate: MentionSuggestion) -> some View {
        switch candidate {
        case .assistant(let name):
            // Its display name and not "@ai": the list offers PEOPLE by the
            // name the family knows them by, and the assistant is offered
            // the same way (protocol.md). The glyph says it is not one.
            HStack(spacing: 4) {
                Image(systemName: "sparkles")
                    .foregroundStyle(.tint)
                    .accessibilityHidden(true)
                Text(verbatim: name)
            }
        case .member(let member):
            Text(verbatim: "@" + member.name)
        }
    }
}
