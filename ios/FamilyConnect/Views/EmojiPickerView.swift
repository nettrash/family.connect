//
//  EmojiPickerView.swift
//  FamilyConnect
//
//  The full reaction picker. A pick calls the parent, which toggles the
//  message reaction and dismisses the presentation. The catalogue tests
//  enforce the server's 32-byte UTF-8 limit.
//

import SwiftUI

#if os(macOS)
import AppKit
#endif

/// The full reaction picker shared by the iOS and macOS presentations.
struct EmojiPickerView: View {
    let account: ReactionRecents.Account
    #if os(iOS)
    var onSearchFocus: () -> Void = {}
    #endif
    let onPick: (String) -> Void

    private enum SectionID: Hashable {
        case recent, category(String), results
    }

    private struct EmojiSection: Identifiable {
        let id: SectionID
        let label: Text
        let emojis: [String]
        var firstCell: CellID? { emojis.first.map { CellID(section: id, emoji: $0) } }
    }

    private struct CellID: Hashable {
        let section: SectionID
        let emoji: String
    }

    private enum Focus: Hashable {
        case search, cell(CellID)
    }

    @State private var query = ""
    @State private var search: ReactionEmojiSearch?
    @State private var matches: [String] = []
    @State private var recent: [String] = []
    @State private var requestedSection: SectionID?
    @State private var picked = false
    @FocusState private var focus: Focus?
    #if os(macOS)
    @State private var pendingCell: CellID?
    private static let columnCount = 8
    #endif

    private var isSearching: Bool { !query.allSatisfy(\.isWhitespace) }

    private var categories: [EmojiSection] {
        let recents = recent.isEmpty ? [] : [
            EmojiSection(id: .recent, label: Text("Recent emoji"), emojis: recent)
        ]
        return recents + EmojiCatalog.categories.map { category in
            EmojiSection(id: .category(category.id), label: Text(verbatim: category.localizedName),
                         emojis: category.emoji)
        }
    }

    private var columns: [GridItem] {
        #if os(macOS)
        Array(repeating: GridItem(.flexible(minimum: 44), spacing: 4), count: Self.columnCount)
        #else
        [GridItem(.adaptive(minimum: 44), spacing: 4)]
        #endif
    }

    var body: some View {
        let categories = categories
        let sections = isSearching
            ? [EmojiSection(id: .results, label: Text("Search results"), emojis: matches)]
            : categories

        ScrollViewReader { proxy in
            VStack(spacing: 0) {
                searchBar(first: sections.first?.firstCell)
                Divider()
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 4, pinnedViews: [.sectionHeaders]) {
                        Color.clear.frame(height: 0).id("reactionPickerTop")
                            .accessibilityHidden(true)
                        if isSearching && search == nil {
                            ProgressView()
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 32)
                        } else if isSearching && matches.isEmpty {
                            Text("No emoji found")
                                .foregroundStyle(.secondary)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 32)
                        } else {
                            ForEach(sections) { section in
                                Section {
                                    LazyVGrid(columns: columns, spacing: 4) {
                                        ForEach(section.emojis, id: \.self) { emoji in
                                            let cell = CellID(section: section.id, emoji: emoji)
                                            emojiButton(cell).id(cell)
                                        }
                                    }
                                } header: {
                                    section.label
                                        .font(.footnote.weight(.semibold))
                                        .foregroundStyle(.secondary)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .padding(.vertical, 6)
                                        .background(.bar)
                                        .accessibilityAddTraits(.isHeader)
                                }
                                .id(section.id)
                            }
                        }
                    }
                    .padding(.horizontal, 12)
                }
                #if os(iOS)
                .scrollDismissesKeyboard(.never)
                #endif
                Divider()
                categoryBar(categories)
            }
            .onChange(of: query) {
                updateMatches()
                if requestedSection == nil { proxy.scrollTo("reactionPickerTop", anchor: .top) }
            }
            .task(id: requestedSection) {
                guard let id = requestedSection else { return }
                proxy.scrollTo(id, anchor: .top)
                #if os(macOS)
                if let cell = categories.first(where: { $0.id == id })?.firstCell {
                    pendingCell = cell
                }
                #endif
                requestedSection = nil
            }
            #if os(macOS)
            .onChange(of: focus) {
                if let pendingCell, focus == .cell(pendingCell) { self.pendingCell = nil }
            }
            .background {
                MacReactionPickerArrowKeys { key in
                    switch focus {
                    case .search where key == .downArrow:
                        guard let cell = sections.first?.firstCell, !Self.hasMarkedText else { return false }
                        moveFocus(to: cell, proxy: proxy)
                    case .cell(let cell):
                        navigate(from: cell, key: key, sections: sections, proxy: proxy)
                    default:
                        return false
                    }
                    return true
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
            #else
            .onChange(of: focus) {
                if focus == .search { onSearchFocus() }
            }
            #endif
        }
        .task {
            recent = ReactionRecents().choices(for: account)
            #if os(macOS)
            focus = .search
            #endif
            let index = await ReactionEmojiSearch.bundled.value
            guard !Task.isCancelled else { return }
            search = index
            updateMatches()
        }
    }

    private func updateMatches() {
        matches = isSearching ? search?.search(query).map(\.emoji) ?? [] : []
    }

    private func searchBar(first: CellID?) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            TextField("Search emoji", text: $query)
                .textFieldStyle(.plain)
                .focused($focus, equals: .search)
                .accessibilityIdentifier("reactionSearch")
                #if os(macOS)
                .onSubmit {
                    if isSearching, let first, !Self.hasMarkedText { pick(first.emoji) }
                }
                #else
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .onSubmit { focus = nil }
                #endif

            if !query.isEmpty {
                Button {
                    query = ""
                    focus = .search
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(12)
    }

    private func categoryBar(_ categories: [EmojiSection]) -> some View {
        HStack(spacing: 2) {
            ForEach(categories) { section in
                Button {
                    requestedSection = section.id
                    query = ""
                } label: {
                    Group {
                        if section.id == .recent {
                            Image(systemName: "clock")
                        } else {
                            Text(verbatim: section.emojis.first ?? "")
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 36)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(section.label)
                #if os(macOS)
                .help(section.label)
                #endif
            }
        }
        .padding(.horizontal, 8)
    }

    private func emojiButton(_ cell: CellID) -> some View {
        Button {
            pick(cell.emoji)
        } label: {
            Text(verbatim: cell.emoji)
                .font(.system(size: 30))
                .frame(maxWidth: .infinity, minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(verbatim: search?.name(for: cell.emoji) ?? cell.emoji))
        .accessibilityIdentifier("reactionEmoji-\(cell.emoji)")
        #if os(macOS)
        .focusable(interactions: .edit)
        .focused($focus, equals: .cell(cell))
        .background {
            if focus == .cell(cell) {
                RoundedRectangle(cornerRadius: 6).strokeBorder(Color.accentColor)
            }
        }
        .task(id: pendingCell == cell) {
            guard pendingCell == cell else { return }
            await Task.yield()
            guard pendingCell == cell else { return }
            focus = .cell(cell)
        }
        .onKeyPress(.return) {
            pick(cell.emoji)
            return .handled
        }
        #endif
    }

    private func pick(_ emoji: String) {
        guard !picked, account == ReactionRecents.Account.current else { return }
        picked = true
        ReactionRecents().remember(emoji, for: account)
        onPick(emoji)
    }

    #if os(macOS)
    static var hasMarkedText: Bool {
        (NSApp.keyWindow?.firstResponder as? any NSTextInputClient)?.hasMarkedText() == true
    }

    private func moveFocus(to cell: CellID, proxy: ScrollViewProxy) {
        if focus == .search {
            focus = nil
            NSApp.keyWindow?.makeFirstResponder(nil)
        }
        pendingCell = cell
        proxy.scrollTo(cell)
    }

    private func navigate(from cell: CellID, key: KeyEquivalent, sections: [EmojiSection], proxy: ScrollViewProxy) {
        guard let index = sections.firstIndex(where: { $0.id == cell.section }),
              let position = sections[index].emojis.firstIndex(of: cell.emoji)
        else { return }
        let section = sections[index]
        let columns = Self.columnCount
        let step: Int
        switch key {
        case .leftArrow: step = -1
        case .rightArrow: step = 1
        case .upArrow: step = -columns
        default: step = columns
        }

        let next = position + step
        if section.emojis.indices.contains(next) {
            moveFocus(to: CellID(section: section.id, emoji: section.emojis[next]), proxy: proxy)
            return
        }
        let nextSection = index + (step < 0 ? -1 : 1)
        guard sections.indices.contains(nextSection) else {
            if key == .upArrow { focus = .search }
            return
        }
        let adjacent = sections[nextSection]
        let column = position % columns
        let last = adjacent.emojis.count - 1
        let target: Int
        switch key {
        case .upArrow: target = min(last - last % columns + column, last)
        case .downArrow: target = min(column, last)
        case .leftArrow: target = last
        default: target = 0
        }
        moveFocus(to: CellID(section: adjacent.id, emoji: adjacent.emojis[target]), proxy: proxy)
    }
    #endif
}
