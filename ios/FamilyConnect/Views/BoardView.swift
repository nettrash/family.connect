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
//  about pixels. The fraction is the note's TOP-LEFT corner, as on the Mac
//  and Android, and it is drawn CLAMPED inside the board: a stored 0.98
//  would otherwise hang a note off the right edge, and a large note off
//  most of the way. Adding a drag to that unclamped origin was worse — a
//  note at the edge did not move until the finger had travelled the
//  overhang. So the drawn origin is clamp(stored) + drag, clamped again,
//  and the fraction reported on release is read back from where it is
//  DRAWN, so what was dropped is what gets stored.
//
//  Size is a name — small, medium, large — chosen by the author with the
//  text and colour, and drawn at the phone's metrics (NoteSize.swift).
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
    var size: NoteSize
    /// The raw wire name the entity holds, nil for a new note. Kept apart
    /// from `size` so an edit that leaves the picker alone sends no size —
    /// a name this client does not know shows as medium but is not saved
    /// as medium (NoteSize.patchName).
    var storedSize: String?
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
                            isHiddenByBlock: MessagePresentation.isNoteHiddenByBlock(
                                authorID: note.authorID,
                                blockedUserIDs: coordinator.blockedUserIDs,
                                currentUserID: currentUserID),
                            boardSize: geometry.size,
                            onMoved: { fraction in
                                Task { await coordinator.updateNote(id: note.noteID, x: fraction.x, y: fraction.y) }
                            },
                            onTap: {
                                editing = NoteDraft(
                                    noteID: note.noteID,
                                    text: note.text,
                                    color: note.color,
                                    size: NoteSize(name: note.size),
                                    storedSize: note.size,
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
                        .keyboardShortcut(.cancelAction)
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
                            size: .medium,
                            storedSize: nil,
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
                    onSave: { text, color, size in
                        save(draft: draft, text: text, color: color, size: size)
                    },
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

    private func save(draft: NoteDraft, text: String, color: String, size: NoteSize) {
        editing = nil
        Task {
            if let id = draft.noteID {
                await coordinator.updateNote(
                    id: id, text: text, color: color,
                    size: size.patchName(replacing: draft.storedSize))
            } else {
                _ = await coordinator.addNote(
                    text: text, color: color, size: size.name, x: draft.x, y: draft.y)
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
    /// Its author is blocked, so the note draws the placeholder and no
    /// author line — content included (docs/protocol.md, "Board").
    let isHiddenByBlock: Bool
    let boardSize: CGSize
    let onMoved: (CGPoint) -> Void
    let onTap: () -> Void

    @State private var drag: CGSize = .zero
    /// A peek, not a setting: per note, per device, never on the wire and
    /// never stored, and gone on the next launch.
    @State private var isRevealed = false

    private var isHidden: Bool { isHiddenByBlock && !isRevealed }

    /// A top-left corner held inside the board, so no part of the note is
    /// off-screen whatever its size.
    private static func clamp(_ point: CGPoint, side: CGFloat, board: CGSize) -> CGPoint {
        CGPoint(
            x: min(max(point.x, 0), max(board.width - side, 0)),
            y: min(max(point.y, 0), max(board.height - side, 0)))
    }

    /// The iPad's sticker is bigger than the phone's — the same 132pt
    /// medium that fills a phone's board was a stamp on a 13-inch canvas
    /// (the Mac's medium is a 150pt card on a window a third the size).
    /// The STEP is still the wire's name; only the points scale. Applied
    /// here rather than in NoteSize, which is shared and pure, and read
    /// once: the idiom never changes while the app runs.
    private static let noteScale: CGFloat = UIDevice.current.userInterfaceIdiom == .pad ? 1.45 : 1

    var body: some View {
        let size = NoteSize(name: note.size)
        let side = size.side * Self.noteScale
        // Where it is drawn RIGHT NOW: the stored corner, held inside the
        // board, plus whatever the drag has moved it, held inside again.
        // Clamping only on release would let a note be dragged off the
        // edge and then snap back.
        let origin = Self.clamp(
            CGPoint(x: note.x * boardSize.width, y: note.y * boardSize.height),
            side: side, board: boardSize)
        let drawn = Self.clamp(
            CGPoint(x: origin.x + drag.width, y: origin.y + drag.height),
            side: side, board: boardSize)

        VStack(alignment: .leading, spacing: 6) {
            (isHidden ? Text("Hidden — blocked member") : Text(note.text))
                .font(size.font)
                .foregroundStyle(.black.opacity(isHidden ? 0.45 : 0.85))
                .italic(isHidden)
                .lineLimit(size.lineLimit)
            Spacer(minLength: 0)
            // No author line at all while hidden — not an empty one, which
            // would still say a note came from somebody.
            if !isHidden {
                Text(authorName)
                    .font(.caption2)
                    .foregroundStyle(.black.opacity(0.5))
            }
        }
        .padding(10)
        .frame(width: side, height: side, alignment: .topLeading)
        .background(NoteColor.swiftUI(note.color), in: RoundedRectangle(cornerRadius: 10))
        .shadow(color: .black.opacity(drag == .zero ? 0.12 : 0.25), radius: drag == .zero ? 3 : 10, y: 2)
        .rotationEffect(.degrees(Self.tilt(for: note.noteID)))
        .scaleEffect(drag == .zero ? 1 : 1.04)
        // `position` places the CENTRE; the stored fraction is the corner.
        .position(x: drawn.x + side / 2, y: drawn.y + side / 2)
        .animation(.spring(duration: 0.2), value: drag == .zero)
        .gesture(
            DragGesture()
                .onChanged { drag = $0.translation }
                .onEnded { _ in
                    // Read back from where it is DRAWN, clamped so a note
                    // dropped past the edge sticks to the edge — matching
                    // what the server would do anyway, and what the Mac
                    // does. Deriving the fraction from the raw translation
                    // would store a position the note was never at.
                    let width = max(boardSize.width, 1)
                    let height = max(boardSize.height, 1)
                    let x = min(max(drawn.x / width, 0), 1)
                    let y = min(max(drawn.y / height, 0), 1)
                    drag = .zero
                    onMoved(CGPoint(x: x, y: y))
                })
        // The FIRST tap on a hidden note reveals it and does nothing
        // else. Falling through to `onTap` would open the note editor,
        // which draws the very text the note is hiding.
        .onTapGesture { if isHidden { isRevealed = true } else { onTap() } }
        // A bare gesture publishes no accessibility action — measured, see
        // ZZAXProbeTests — so the note declares its own.
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onTap() }
    }

    /// What VoiceOver reads. A hidden note announces that it is hidden and
    /// stops: reading the text aloud would defeat the whole thing for the
    /// one reader most dependent on the label being honest.
    private var accessibilityText: Text {
        if isHidden { return Text("Hidden note from a blocked member") }
        return isMine
            ? Text("Your note: \(note.text)")
            : Text("Note from \(authorName): \(note.text)")
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
    let onSave: (String, String, NoteSize) -> Void
    let onDelete: (() -> Void)?

    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var color: String
    @State private var size: NoteSize
    @State private var confirmDelete = false

    init(
        draft: NoteDraft,
        canEdit: Bool,
        authorName: String,
        onSave: @escaping (String, String, NoteSize) -> Void,
        onDelete: (() -> Void)?
    ) {
        self.draft = draft
        self.canEdit = canEdit
        self.authorName = authorName
        self.onSave = onSave
        self.onDelete = onDelete
        _text = State(initialValue: draft.text)
        _color = State(initialValue: draft.color)
        _size = State(initialValue: draft.size)
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
                                        // 36, not 30: a tap target near
                                        // the 44pt guideline, and room for
                                        // the checkmark on the chosen one.
                                        .frame(width: 36, height: 36)
                                        .overlay {
                                            if color == name {
                                                Image(systemName: "checkmark")
                                                    .font(.caption.weight(.bold))
                                                    .foregroundStyle(.black.opacity(0.7))
                                            }
                                        }
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
                    // Size sits with text and colour: it is the author's
                    // call how loudly a note speaks, so a reader of someone
                    // else's note never sees this section.
                    Section("Size") {
                        Picker("Size", selection: $size) {
                            ForEach(NoteSize.allCases) { size in
                                Text(size.title).tag(size)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
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
                        .keyboardShortcut(.cancelAction)
                }
                if canEdit {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") { onSave(text, color, size) }
                            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
    }
}


#endif
