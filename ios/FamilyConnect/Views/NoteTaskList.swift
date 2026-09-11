//
//  NoteTaskList.swift
//  FamilyConnect
//
//  A task list on the board: what the STICKER draws of it, and the rows in
//  the note somebody has opened (docs/protocol.md, "Board").
//
//  THE SPLIT IS THE POINT. The author writes the lines — that is an edit,
//  like the title — and ANY member ticks them, which is the shared act an
//  event's answer is. So the rows below carry both: a box everybody may
//  tap, and, for the author, the words beside it.
//
//  And the tick is not on the WALL. A sticker's whole face is a drag
//  handle, and a row of small boxes on it would be a wall nobody could
//  tidy: the wall shows what is done, and the tap that ticks is one tap
//  further on.
//
//  Web counterpart: `wall_list` and the sheet's `task-list` in
//  web/src/views/board.rs.
//  Android counterpart: `NoteTasks` in ui/board/BoardScreen.kt.
//
import SwiftUI

/// The block a task list draws under its title on the wall: the first
/// lines with their state, and then how many are left.
///
/// Nothing here takes a tap, and nothing here is read out on its own: the
/// sticker's own label already says what the note says.
struct NoteTaskBlock: View {
    let items: [TaskItemDTO]

    var body: some View {
        let drawn = BoardTasks.drawn(of: items.count)
        VStack(alignment: .leading, spacing: 1) {
            ForEach(items.prefix(drawn.shown), id: \.id) { item in
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Image(systemName: item.done ? "checkmark.square" : "square")
                        .font(.caption2)
                    Text(item.text)
                        .strikethrough(item.done)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                .font(.caption2)
                .foregroundStyle(.black.opacity(item.done ? 0.45 : 0.7))
            }
            if drawn.left > 0 {
                Text("+\(drawn.left) more")
                    .font(.caption2)
                    .italic()
                    .foregroundStyle(.black.opacity(0.45))
            }
        }
        .accessibilityHidden(true)
    }
}

/// One line as the AUTHOR is writing it: the server's id where there is
/// one, and the words.
///
/// `itemID` is what carries a line's TICK through a rewrite, and the local
/// id is what a `ForEach` needs — a line nobody has saved has no server id
/// yet, and two of them would otherwise be the same row
/// (docs/protocol.md, "Board").
struct DraftTaskLine: Identifiable, Equatable {
    let id = UUID()
    var itemID: Int64?
    var text: String

    init(itemID: Int64? = nil, text: String = "") {
        self.itemID = itemID
        self.text = text
    }

    /// What a save sends: the lines that say something, trimmed.
    static func written(_ lines: [DraftTaskLine]) -> [APIClient.TaskLineRequest] {
        lines
            .filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map {
                APIClient.TaskLineRequest(
                    id: $0.itemID,
                    text: $0.text.trimmingCharacters(in: .whitespacesAndNewlines))
            }
    }
}

/// One line of a list, in the note somebody has opened.
///
/// The box is a Button and not a Toggle: a Toggle's binding would make the
/// tick look like this device's own state, and it is the family's — the
/// server's answer is what it goes back to.
struct NoteTaskRow: View {
    /// Nil on a line nobody has saved: there is nothing to tick yet, and
    /// the box says so by being disabled rather than by not being there —
    /// a row that grew a box on save would jump under the finger.
    let itemID: Int64?
    /// What this device is showing right now, which is the tap's own
    /// answer while one is in flight.
    let done: Bool
    let canEdit: Bool
    let onTick: (Bool) -> Void
    let onRemove: (() -> Void)?
    @Binding var text: String

    var body: some View {
        HStack(spacing: 10) {
            Button {
                onTick(!done)
            } label: {
                Image(systemName: done ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(done ? Color.accentColor : Color.secondary)
            }
            .buttonStyle(.plain)
            .disabled(itemID == nil)
            .accessibilityLabel(
                text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? String(localized: "Done")
                    : text)
            .accessibilityAddTraits(done ? [.isButton, .isSelected] : .isButton)
            if canEdit {
                TextField("Thing to do", text: $text)
                    .textFieldStyle(.plain)
                if let onRemove {
                    Button(role: .destructive, action: onRemove) {
                        Image(systemName: "minus.circle")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Remove")
                }
            } else {
                Text(text)
                    .strikethrough(done)
                    .foregroundStyle(done ? .secondary : .primary)
            }
        }
    }
}
