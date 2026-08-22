//
//  MacBoardView.swift
//  FamilyConnect
//
//  The family board on the Mac: sticker notes on a wall.
//
//  Positions are FRACTIONS of the board (0–1), not pixels — the same
//  values the phone stores, so a note dragged on a Mac lands in the same
//  relative place on everyone's screen whatever the window size
//  (docs/protocol.md, "Board").
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
    @Environment(\.dismiss) private var dismiss

    @Query(sort: [SortDescriptor(\NoteEntity.createdAt)]) private var notes: [NoteEntity]

    @State private var editing: NoteEntity?
    @State private var composing = false
    @State private var draftText = ""
    @State private var draftColor = NoteColor.palette.first ?? "yellow"

    var body: some View {
        VStack(spacing: 0) {
            GeometryReader { geometry in
                ZStack(alignment: .topLeading) {
                    Color(nsColor: .underPageBackgroundColor)
                    ForEach(notes) { note in
                        MacNoteView(
                            note: note,
                            board: geometry.size,
                            isMine: note.authorID == coordinator.currentUserID,
                            onMove: { x, y in
                                Task { await coordinator.updateNote(id: note.noteID, x: x, y: y) }
                            },
                            onEdit: { editing = note },
                            onDelete: {
                                Task { _ = await coordinator.deleteNote(id: note.noteID) }
                            })
                    }
                    if notes.isEmpty {
                        ContentUnavailableView(
                            "The board is empty",
                            systemImage: "doc.text",
                            description: Text("Add a note — everyone in the family sees it."))
                        .frame(width: geometry.size.width, height: geometry.size.height)
                    }
                }
            }
            Divider()
            HStack {
                Button {
                    draftText = ""
                    draftColor = NoteColor.palette.randomElement() ?? "yellow"
                    composing = true
                } label: {
                    Label("Add Note", systemImage: "plus")
                }
                .keyboardShortcut("n", modifiers: .command)
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(12)
        }
        .frame(minWidth: 640, minHeight: 480)
        .task { await coordinator.loadBoard() }
        .sheet(isPresented: $composing) {
            MacNoteEditor(text: $draftText, color: $draftColor, title: "New Note") {
                Task {
                    // Dropped near the middle with a little scatter, so a
                    // run of new notes does not stack into one pile.
                    _ = await coordinator.addNote(
                        text: draftText,
                        color: draftColor,
                        x: Double.random(in: 0.25...0.65),
                        y: Double.random(in: 0.25...0.65))
                }
            }
        }
        .sheet(item: $editing) { note in
            MacNoteEditorForExisting(note: note)
        }
    }
}

/// One sticker: positioned by fraction, dragged locally, committed once.
private struct MacNoteView: View {
    let note: NoteEntity
    let board: CGSize
    let isMine: Bool
    let onMove: (Double, Double) -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    @State private var drag: CGSize = .zero
    @State private var committing = false

    private static let size = CGSize(width: 150, height: 110)

    var body: some View {
        let origin = CGPoint(
            x: note.x * board.width - Self.size.width / 2,
            y: note.y * board.height - Self.size.height / 2)

        VStack(alignment: .leading, spacing: 4) {
            Text(note.text)
                .font(.callout)
                .lineLimit(4)
            Spacer(minLength: 0)
        }
        .padding(8)
        .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
        .background(NoteColor.swiftUI(note.color), in: RoundedRectangle(cornerRadius: 6))
        .shadow(radius: 2, y: 1)
        // A tiny id-derived tilt: a wall of perfectly square notes reads
        // as a table, not a pinboard. Derived, so it never changes.
        .rotationEffect(.degrees(Double(note.noteID % 7) - 3))
        .offset(x: origin.x + drag.width, y: origin.y + drag.height)
        .gesture(
            DragGesture()
                .onChanged { drag = $0.translation }
                .onEnded { value in
                    let x = ((origin.x + value.translation.width + Self.size.width / 2)
                        / board.width).clampedToBoard()
                    let y = ((origin.y + value.translation.height + Self.size.height / 2)
                        / board.height).clampedToBoard()
                    committing = true
                    onMove(x, y)
                    // The local offset is NOT zeroed here: it is released
                    // when the authoritative position lands, or the note
                    // jumps back for a frame.
                })
        .onChange(of: note.x) { _, _ in drag = .zero; committing = false }
        .onChange(of: note.y) { _, _ in drag = .zero; committing = false }
        .contextMenu {
            if isMine {
                Button("Edit…", action: onEdit)
                Button("Delete", role: .destructive, action: onDelete)
            } else {
                // Anyone may MOVE a note; only its author may change it.
                Text("Written by someone else")
            }
        }
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
    let title: String
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
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
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

    var body: some View {
        MacNoteEditor(text: $text, color: $color, title: "Edit Note") {
            Task { await coordinator.updateNote(id: note.noteID, text: text, color: color) }
        }
        .onAppear {
            text = note.text
            color = note.color
        }
    }
}

#endif
