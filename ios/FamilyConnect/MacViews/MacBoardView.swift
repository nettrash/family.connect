//
//  MacBoardView.swift
//  FamilyConnect
//
//  The family board on the Mac: sticker notes on a wall.
//
//  Positions are FRACTIONS of the board (0–1), not pixels — the same
//  values the phone stores, so a note dragged on a Mac lands in the same
//  relative place on everyone's screen whatever the window size
//  (docs/protocol.md, "Board"). The fraction is the note's TOP-LEFT
//  corner, as the protocol says and as the phones draw it; an earlier
//  version of this file treated it as the centre, which put the same note
//  half a sticker apart on a Mac and a phone. The corner is drawn CLAMPED
//  inside the board, so a stored 0.98 hugs the edge rather than hanging
//  off it, and the fraction read back on release is where the note was
//  drawn, not where the arithmetic would have put it.
//
//  Size is a name — small, medium, large — chosen by the author with the
//  text and colour, and drawn at the Mac's metrics (NoteSize.swift): a
//  landscape card, bigger than the phone's square sticker at every step.
//
//  Dragging reports the fraction on RELEASE, not every frame: a drag is
//  one intent, and a note that fanned out sixty times a second would be a
//  sixty-fold write amplification on the family's server. The local offset
//  is held until the authoritative position arrives, because zeroing it on
//  release snaps the note back for a frame.
//
//  Moving is anyone's; rewriting and deleting are the author's — enforced
//  by WHICH FIELDS the PATCH carries, so a move must send only x/y.
//

#if os(macOS)

import SwiftData
import SwiftUI

struct MacBoardView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    /// Whether this window is the one in front of somebody, the same test
    /// a conversation window uses before it dares call a message read
    /// (ChatPresence). A board window restored at login behind everything
    /// else has shown nobody anything.
    @Environment(\.controlActiveState) private var windowActivation

    @Query(sort: [SortDescriptor(\NoteEntity.createdAt)]) private var notes: [NoteEntity]
    @Query private var members: [MemberEntity]

    @State private var editing: NoteEntity?
    @State private var composing = false
    @State private var draftText = ""
    @State private var draftColor = NoteColor.palette.first ?? "yellow"
    @State private var draftSize = NoteSize.medium

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .topLeading) {
                Color(nsColor: .underPageBackgroundColor)
                ForEach(notes) { note in
                    MacNoteView(
                        note: note,
                        board: geometry.size,
                        isMine: note.authorID == coordinator.currentUserID,
                        authorName: displayName(for: note.authorID),
                        isHiddenByBlock: MessagePresentation.isNoteHiddenByBlock(
                            authorID: note.authorID,
                            blockedUserIDs: coordinator.blockedUserIDs,
                            currentUserID: coordinator.currentUserID),
                        onMove: { x, y in
                            _ = await coordinator.updateNote(id: note.noteID, x: x, y: y)
                        },
                        onResize: { size in
                            Task { await coordinator.updateNote(id: note.noteID, size: size.name) }
                        },
                        onEdit: { editing = note },
                        onDelete: {
                            Task { _ = await coordinator.deleteNote(id: note.noteID) }
                        })
                }
                if notes.isEmpty {
                    ContentUnavailableView(
                        "The board is empty",
                        systemImage: "square.grid.2x2",
                        description: Text("Add a note — everyone in the family sees it."))
                    .frame(width: geometry.size.width, height: geometry.size.height)
                }
            }
        }
        .frame(minWidth: 480, minHeight: 360)
        .navigationTitle("Board")
        .toolbar {
            ToolbarItem {
                Button {
                    draftText = ""
                    draftColor = NoteColor.palette.randomElement() ?? "yellow"
                    draftSize = .medium
                    composing = true
                } label: {
                    Label("Add Note", systemImage: "plus")
                }
                .keyboardShortcut("n", modifiers: .command)
                .help("Add a note")
            }
        }
        .task { await coordinator.loadBoard() }
        // The board on the Mac is a WINDOW, not a sheet: it is opened once
        // and left open, and notes land on a wall somebody is already
        // looking at. Marking seen only where it is OPENED (MacChatView's
        // toolbar) therefore left every one of those counted as new until
        // the next click on a window that was never closed. So the window
        // marks what it is showing, whenever what it shows changes and
        // whenever it becomes the front one.
        .onChange(of: boardMark, initial: true) { _, _ in markSeenIfFrontmost() }
        .onChange(of: windowActivation, initial: true) { _, _ in markSeenIfFrontmost() }
        .sheet(isPresented: $composing) {
            MacNoteEditor(text: $draftText, color: $draftColor, size: $draftSize, title: "New Note") {
                Task {
                    // Dropped near the middle with a little scatter, so a
                    // run of new notes does not stack into one pile.
                    _ = await coordinator.addNote(
                        text: draftText,
                        color: draftColor,
                        size: draftSize.name,
                        x: Double.random(in: 0.25...0.65),
                        y: Double.random(in: 0.25...0.65))
                }
            }
        }
        .sheet(item: $editing) { note in
            MacNoteEditorForExisting(note: note)
        }
    }

    /// What the marks WOULD be if this wall counted as shown — an
    /// Equatable value, so `onChange` fires on a note arriving, on one
    /// being rewritten, and on nothing else. A drag changes no part of it.
    private var boardMark: BoardBadge.Marks {
        BoardBadge.marksAfterShowing(notes: notes, marks: .zero)
    }

    private func markSeenIfFrontmost() {
        guard windowActivation == .key else { return }
        AppSettings.boardMarks = BoardBadge.marksAfterShowing(
            notes: notes, marks: AppSettings.boardMarks)
    }

    private func displayName(for userID: Int64) -> String {
        if userID == coordinator.currentUserID { return String(localized: "You") }
        return members.first { $0.userID == userID }?.resolvedDisplayName
            ?? String(localized: "Someone")
    }
}

/// One sticker: positioned by fraction, dragged locally, committed once.
private struct MacNoteView: View {
    let note: NoteEntity
    let board: CGSize
    let isMine: Bool
    let authorName: String
    /// Its author is blocked, so the note draws the placeholder and no
    /// author line — content included (docs/protocol.md, "Board").
    let isHiddenByBlock: Bool
    /// Awaited, so the sticker knows when the move is over (see the drag).
    let onMove: (Double, Double) async -> Void
    let onResize: (NoteSize) -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    @State private var drag: CGSize = .zero
    @State private var committing = false
    /// A peek, not a setting: per note, per device, never on the wire and
    /// never stored, and gone on the next launch.
    @State private var isRevealed = false

    private var isHidden: Bool { isHiddenByBlock && !isRevealed }

    private var isDragging: Bool { drag != .zero && !committing }

    /// Where a note's fractional position puts its top-left corner, held
    /// inside the board so no part of it is off-screen — which depends on
    /// the note's size, a large one running out of room sooner.
    private static func origin(x: Double, y: Double, size: CGSize, board: CGSize) -> CGPoint {
        clamp(CGPoint(x: x * board.width, y: y * board.height), size: size, board: board)
    }

    private static func clamp(_ point: CGPoint, size: CGSize, board: CGSize) -> CGPoint {
        CGPoint(
            x: min(max(point.x, 0), max(board.width - size.width, 0)),
            y: min(max(point.y, 0), max(board.height - size.height, 0)))
    }

    var body: some View {
        let noteSize = NoteSize(name: note.size)
        let size = noteSize.frame
        // Where it is drawn RIGHT NOW: its stored position plus whatever
        // the drag has moved it, held inside the board either way. Clamping
        // only on release would let a note be dragged off the edge and then
        // snap back.
        let origin = Self.origin(x: note.x, y: note.y, size: size, board: board)
        let position = Self.clamp(
            CGPoint(x: origin.x + drag.width, y: origin.y + drag.height),
            size: size, board: board)

        VStack(alignment: .leading, spacing: 4) {
            (isHidden ? Text("Hidden — blocked member") : Text(note.text))
                .font(noteSize.font)
                // Forced ink, matching BoardView: the pastels are fixed
                // light colors in both appearances, so .primary’s dark-mode
                // white was unreadable on them.
                .foregroundStyle(.black.opacity(isHidden ? 0.45 : 0.85))
                .italic(isHidden)
                .lineLimit(noteSize.lineLimit)
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
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        .background(NoteColor.swiftUI(note.color), in: RoundedRectangle(cornerRadius: 8))
        // Lifted off the wall while it is in hand — the same cue the phone
        // gives, and the only feedback a cursor drag has.
        .shadow(color: .black.opacity(isDragging ? 0.28 : 0.12),
                radius: isDragging ? 10 : 3, y: 2)
        .scaleEffect(isDragging ? 1.04 : 1)
        .animation(.easeOut(duration: 0.12), value: isDragging)
        // A tiny id-derived tilt: a wall of perfectly square notes reads
        // as a table, not a pinboard. Derived, so it never changes.
        .rotationEffect(.degrees(Double(note.noteID % 7) - 3))
        .offset(x: position.x, y: position.y)
        .gesture(
            DragGesture()
                .onChanged { drag = $0.translation }
                .onEnded { _ in
                    // Read back from where it is DRAWN, so what was dropped
                    // is what gets stored — deriving the fraction from the
                    // raw translation would save a position the note was
                    // never actually at. The corner, not the centre: that
                    // is what the phones store, and what protocol.md says.
                    let x = (position.x / max(board.width, 1)).clampedToBoard()
                    let y = (position.y / max(board.height, 1)).clampedToBoard()
                    committing = true
                    // The local offset is NOT zeroed here: it is released
                    // when the authoritative position lands, or the note
                    // jumps back for a frame. It is released when the move
                    // RETURNS as well: a drop that reproduces the stored
                    // fraction (a note on the edge pushed further out) is a
                    // server no-op, and a failed PATCH changes nothing
                    // either — neither moves `note.x`, so without this the
                    // note would stay "committing" and the next drag would
                    // show no lift. On success the coordinator has applied
                    // the reply before returning, so this runs after the
                    // position landed and changes nothing.
                    Task {
                        await onMove(x, y)
                        drag = .zero
                        committing = false
                    }
                })
        .onChange(of: note.x) { _, _ in drag = .zero; committing = false }
        .onChange(of: note.y) { _, _ in drag = .zero; committing = false }
        // A note is draggable, and on a Mac only the cursor says so.
        .hoverCursor(.openHand)
        // A click on a note is otherwise a no-op here — the Mac edits from
        // the context menu — so this costs nothing and matches the phone.
        .onTapGesture { if isHidden { isRevealed = true } }
        .accessibilityLabel(accessibilityText)
        .contextMenu {
            if isHidden {
                // The one item a hidden note offers. Its size and delete
                // are the author's anyway, and "Written by someone else"
                // below would be a hint about who.
                Button("Reveal") { isRevealed = true }
            } else if isMine {
                Button("Edit…", action: onEdit)
                // A Toggle in a menu is the native checkmarked item, so the
                // current size reads as a state rather than an icon. Each
                // choice PATCHes the size alone — the author's field, so
                // the menu is the author's too.
                Menu("Size") {
                    ForEach(NoteSize.allCases) { size in
                        Toggle(size.title, isOn: Binding(
                            get: { noteSize == size },
                            set: { on in if on { onResize(size) } }))
                    }
                }
                Button("Delete", role: .destructive, action: onDelete)
            } else {
                // Anyone may MOVE a note; only its author may change it.
                Text("Written by someone else")
            }
        }
    }
}

private extension MacNoteView {
    /// What VoiceOver reads. A hidden note announces that it is hidden and
    /// stops: reading the text aloud would defeat the whole thing for the
    /// one reader most dependent on the label being honest.
    var accessibilityText: Text {
        if isHidden { return Text("Hidden note from a blocked member") }
        return isMine
            ? Text("Your note: \(note.text)")
            : Text("Note from \(authorName): \(note.text)")
    }
}

private extension Double {
    /// Out-of-range values are clamped, not rejected: a drag that ends
    /// past the edge should stick, not fail (protocol.md, "Board").
    func clampedToBoard() -> Double { Swift.min(Swift.max(self, 0), 1) }
}

/// Compose a new note.
private struct MacNoteEditor: View {
    @Binding var text: String
    @Binding var color: String
    @Binding var size: NoteSize
    /// A key, not a String: `Text(title)` then goes through the catalog
    /// ("New Note" / "Edit Note") instead of shipping English verbatim.
    let title: LocalizedStringKey
    let onSave: () -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            TextEditor(text: $text)
                .frame(width: 320, height: 120)
                .border(.separator)
            HStack(spacing: 6) {
                ForEach(NoteColor.palette, id: \.self) { name in
                    Circle()
                        .fill(NoteColor.swiftUI(name))
                        .frame(width: 22, height: 22)
                        .overlay(Circle().strokeBorder(
                            name == color ? Color.accentColor : .clear, lineWidth: 2))
                        .onTapGesture { color = name }
                }
            }
            Picker("Size", selection: $size) {
                ForEach(NoteSize.allCases) { size in
                    Text(size.title).tag(size)
                }
            }
            .pickerStyle(.segmented)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    onSave()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(16)
    }
}

/// Rewrite one that already exists — author only, which the caller gates.
private struct MacNoteEditorForExisting: View {
    let note: NoteEntity

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @State private var text: String = ""
    @State private var color: String = "yellow"
    @State private var size: NoteSize = .medium

    var body: some View {
        MacNoteEditor(text: $text, color: $color, size: $size, title: "Edit Note") {
            Task {
                // Size only when the author changed it, so a name this
                // Mac does not know survives a text edit (NoteSize).
                await coordinator.updateNote(
                    id: note.noteID, text: text, color: color,
                    size: size.patchName(replacing: note.size))
            }
        }
        .onAppear {
            text = note.text
            color = note.color
            size = NoteSize(name: note.size)
        }
    }
}

#endif
