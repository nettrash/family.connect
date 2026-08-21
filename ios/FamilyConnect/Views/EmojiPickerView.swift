//
//  EmojiPickerView.swift
//  FamilyConnect
//
//  The full emoji picker behind the floating capsule's "+": every
//  EmojiCatalog category as a pinned section header over a lazy adaptive
//  grid. Picking calls straight out — the parent toggles the reaction
//  and tears the whole presentation down (sheet and capsule both).
//
//  The server accepts any emoji (≤ 32 bytes UTF-8), so nothing here is
//  validated at tap time; the catalog's unit tests hold that invariant.
//

import SwiftUI

struct EmojiPickerView: View {
    let onPick: (String) -> Void

    private let columns = [GridItem(.adaptive(minimum: 44), spacing: 4)]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 4, pinnedViews: [.sectionHeaders]) {
                ForEach(EmojiCatalog.categories) { category in
                    Section {
                        LazyVGrid(columns: columns, spacing: 4) {
                            ForEach(category.emoji, id: \.self) { emoji in
                                Button {
                                    onPick(emoji)
                                } label: {
                                    Text(emoji)
                                        .font(.system(size: 30))
                                        .frame(maxWidth: .infinity, minHeight: 44)
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel(emoji)
                            }
                        }
                    } header: {
                        Text(category.localizedName)
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, 6)
                            .background(.bar)
                    }
                }
            }
            .padding(.horizontal, 12)
        }
    }
}
