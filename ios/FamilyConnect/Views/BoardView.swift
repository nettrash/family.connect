//
//  BoardView.swift
//  FamilyConnect
//
//  The family board: a wall of sticker notes anyone can add to and
//  rearrange (docs/protocol.md, "Board").
//
//  Positions are FRACTIONS of the board, not points, so the wall looks the
//  same on a phone and an iPad — the view multiplies by its own size on the
//  way out and divides on the way in, and nothing but that conversion knows
//  about pixels.
//
//  Two authorship rules, and the UI has to make both legible: anyone may
//  DRAG any note (tidying the wall is shared), but tapping to edit or
//  delete only does something on your own (rewriting someone's words is
//  not). A note you cannot edit still opens — read-only, saying who wrote
//  it — rather than silently ignoring the tap.
//
//  Android counterpart: ui/board/BoardScreen.kt
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import SwiftData
import SwiftUI

/// A note being written or rewritten. `noteID` nil = a new one.
private struct NoteDraft: Identifiable {
    var id: Int64 { noteID ?? -1 }
    var noteID: Int64?
    var text: String
    var color: String
    var x: Double
    var y: Double
    var authorID: Int64
}

struct BoardView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query(sort: \NoteEntity.boardSeq) private var notes: [NoteEntity]
    @Query private var members: [MemberEntity]

    @State private var editing: NoteDraft?
    @State private var loadFailed = false

    private var currentUserID: Int64 { AppSettings.currentUserID ?? -1 }

    var body: some View {
        NavigationStack {
            GeometryReader { geometry in
                ZStack(alignment: .topLeading) {
                    Color(.systemGroupedBackground)
                        .ignoresSafeArea()

                    if notes.isEmpty {
                        ContentUnavailableView(
                            "The board is empty",
                            systemImage: "square.grid.2x2",
                            description: Text("Add a note — everyone in the family sees it."))
                    }

                    ForEach(notes) { note in
                        StickyNote(
                            note: note,
                            isMine: note.authorID == currentUserID,
                            authorName: displayName(for: note.authorID),
                            boardSize: geometry.size,
                            onMoved: { fraction in
                                Task { await coordinator.updateNote(id: note.noteID, x: fraction.x, y: fraction.y) }
                            },
                            onTap: {
                                editing = NoteDraft(
                                    noteID: note.noteID,
                                    text: note.text,
                                    color: note.color,
                                    x: note.x,
                                    y: note.y,
                                    authorID: note.authorID)
                            })
                    }
                }
            }
            .navigationTitle("Board")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        // New notes land near the top-left, offset a little
                        // each time so a burst of them does not stack into
                        // one illegible pile.
                        let slot = Double(notes.count % 6)
                        editing = NoteDraft(
                            noteID: nil,
                            text: "",
                            color: NoteColor.palette.randomElement() ?? "yellow",
                            x: 0.12 + slot * 0.03,
                            y: 0.10 + slot * 0.06,
                            authorID: currentUserID)
                    } label: {
                        Label("Add Note", systemImage: "plus")
                    }
                }
            }
            .sheet(item: $editing) { draft in
                NoteEditor(
                    draft: draft,
                    canEdit: draft.noteID == nil || draft.authorID == currentUserID,
                    authorName: displayName(for: draft.authorID),
                    onSave: { text, color in save(draft: draft, text: text, color: color) },
                    onDelete: draft.noteID.map { id in { delete(id: id) } })
            }
            .task { await coordinator.loadBoard() }
            .refreshable { await coordinator.loadBoard() }
        }
    }

    private func displayName(for userID: Int64) -> String {
        if userID == currentUserID { return String(localized: "You") }
        return members.first { $0.userID == userID }?.resolvedDisplayName
            ?? String(localized: "Someone")
    }

    private func save(draft: NoteDraft, text: String, color: String) {
        editing = nil
        Task {
            if let id = draft.noteID {
                await coordinator.updateNote(id: id, text: text, color: color)
            } else {
                _ = await coordinator.addNote(text: text, color: color, x: draft.x, y: draft.y)
            }
        }
    }

    private func delete(id: Int64) {
        editing = nil
        Task { _ = await coordinator.deleteNote(id: id) }
    }
}

/// One sticker. Drag moves it locally at once and reports the FRACTION on
/// release — the server is told where it ended up, not every frame of how
/// it got there.
private struct StickyNote: View {
    let note: NoteEntity
    let isMine: Bool
    let authorName: String
    let boardSize: CGSize
    let onMoved: (CGPoint) -> Void
    let onTap: () -> Void

    @State private var drag: CGSize = .zero

    private static let side: CGFloat = 132

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(note.text)
                .font(.callout)
                .foregroundStyle(.black.opacity(0.85))
                .lineLimit(5)
            Spacer(minLength: 0)
            Text(authorName)
                .font(.caption2)
                .foregroundStyle(.black.opacity(0.5))
        }
        .padding(10)
        .frame(width: Self.side, height: Self.side, alignment: .topLeading)
        .background(NoteColor.swiftUI(note.color), in: RoundedRectangle(cornerRadius: 10))
        .shadow(color: .black.opacity(drag == .zero ? 0.12 : 0.25), radius: drag == .zero ? 3 : 10, y: 2)
        .rotationEffect(.degrees(Self.tilt(for: note.noteID)))
        .scaleEffect(drag == .zero ? 1 : 1.04)
        .position(
            x: note.x * boardSize.width + Self.side / 2 + drag.width,
            y: note.y * boardSize.height + Self.side / 2 + drag.height)
        .animation(.spring(duration: 0.2), value: drag == .zero)
        .gesture(
            DragGesture()
                .onChanged { drag = $0.translation }
                .onEnded { value in
                    // Report where it landed as a fraction, clamped so a
                    // note dropped past the edge sticks to the edge —
                    // matching what the server would do anyway.
                    let width = max(boardSize.width, 1)
                    let height = max(boardSize.height, 1)
                    let x = min(max((note.x * width + value.translation.width) / width, 0), 1)
                    let y = min(max((note.y * height + value.translation.height) / height, 0), 1)
                    drag = .zero
                    onMoved(CGPoint(x: x, y: y))
                })
        .onTapGesture { onTap() }
        // A bare gesture publishes no accessibility action — measured, see
        // ZZAXProbeTests — so the note declares its own.
        .accessibilityElement(children: .combine)
        .accessibilityLabel(isMine ? "Your note: \(note.text)" : "Note from \(authorName): \(note.text)")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onTap() }
    }

    /// A degree or two of tilt, derived from the id so a note keeps the
    /// same angle for everyone and across launches.
    private static func tilt(for id: Int64) -> Double {
        Double(id % 5) - 2
    }
}

/// The add/edit sheet. Read-only when the note is someone else's: the tap
/// still opens something rather than doing nothing, it just cannot be
/// changed.
private struct NoteEditor: View {
    let draft: NoteDraft
    let canEdit: Bool
    let authorName: String
    let onSave: (String, String) -> Void
    let onDelete: (() -> Void)?

    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var color: String
    @State private var confirmDelete = false

    init(
        draft: NoteDraft,
        canEdit: Bool,
        authorName: String,
        onSave: @escaping (String, String) -> Void,
        onDelete: (() -> Void)?
    ) {
        self.draft = draft
        self.canEdit = canEdit
        self.authorName = authorName
        self.onSave = onSave
        self.onDelete = onDelete
        _text = State(initialValue: draft.text)
        _color = State(initialValue: draft.color)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if canEdit {
                        TextField("Note", text: $text, axis: .vertical)
                            .lineLimit(3...8)
                    } else {
                        Text(draft.text)
                        Text("Written by \(authorName)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                if canEdit {
                    Section("Colour") {
                        HStack(spacing: 12) {
                            ForEach(NoteColor.palette, id: \.self) { name in
                                Button {
                                    color = name
                                } label: {
                                    Circle()
                                        .fill(NoteColor.swiftUI(name))
                                        .frame(width: 30, height: 30)
                                        .overlay(
                                            Circle().strokeBorder(
                                                .primary,
                                                lineWidth: color == name ? 2 : 0))
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel(name)
                                .accessibilityAddTraits(color == name ? [.isButton, .isSelected] : .isButton)
                            }
                        }
                    }
                }
                if let onDelete, canEdit {
                    Section {
                        Button("Delete Note", role: .destructive) { confirmDelete = true }
                    }
                    .confirmationDialog(
                        "Delete this note?",
                        isPresented: $confirmDelete,
                        titleVisibility: .visible
                    ) {
                        Button("Delete", role: .destructive) { onDelete() }
                    }
                }
            }
            .navigationTitle(draft.noteID == nil ? "New Note" : "Note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                if canEdit {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") { onSave(text, color) }
                            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
    }
}


#endif
