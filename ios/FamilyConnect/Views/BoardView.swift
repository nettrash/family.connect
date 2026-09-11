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
import PhotosUI
import SwiftUI

/// A note being written or rewritten. `noteID` nil = a new one.
/// What an event's author changed, handed back with the rest of the note.
/// Empty on every other kind, which is what the save path checks.
struct EventEdit {
    var startsAt: Date?
    var endsAt: Date?
    var place: String
}

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
    var font: NoteFont
    /// The same arrangement one field over (NoteFont.patchName).
    var storedFont: String?
    /// An event's own three, and the kind that makes them meaningful.
    var kind: NoteKind = .text
    var startsAt: Date?
    var endsAt: Date?
    var place: String = ""
    /// What this reader has answered, and everybody's answers, for the
    /// card in the editor.
    var myAnswer: String?
    var rsvps: [RsvpDTO] = []
    /// The members the note names (docs/protocol.md, "Board") — for the
    /// reader's view of it, where a name IS a door.
    var mentions: [MentionDTO] = []
    /// The things to do, as stored: the ids a rewrite keeps and the ticks
    /// the boxes draw (docs/protocol.md, "Board").
    var items: [TaskItemDTO] = []
    var x: Double
    var y: Double
    var authorID: Int64
}

struct BoardView: View {
    /// Where a name in an open note goes: the chat with that member. Nil
    /// where there is no such door — the board is then read-only about
    /// names, which is what the protocol allows (docs/protocol.md,
    /// "Board").
    var onOpenChat: ((Int64) -> Void)?

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query(sort: \NoteEntity.boardSeq) private var notes: [NoteEntity]
    @Query private var members: [MemberEntity]
    /// Pinning a picture: the picker, and the upload it turns into.
    @State private var pickedPhoto: PhotosPickerItem?
    @State private var showPhotoPicker = false
    @State private var pinning = false
    @State private var pinFailure: String?


    @State private var editing: NoteDraft?
    @State private var loadFailed = false

    private var currentUserID: Int64 { AppSettings.currentUserID ?? -1 }

    var body: some View {
        NavigationStack {
            GeometryReader { geometry in
                // THE WALL SCROLLS, because it is taller than the window
                // (docs/protocol.md, "Board"): a wall the size of the
                // window is a wall that fills up. The fractions are read
                // against the WALL, so nothing moves relative to anything
                // else — the bottom of it is simply below the fold.
                let wall = BoardWall.size(visible: geometry.size)
                ScrollView(.vertical) {
                ZStack(alignment: .topLeading) {
                    BoardGround()

                    if notes.isEmpty {
                        ContentUnavailableView(
                            "The board is empty",
                            systemImage: "square.grid.2x2",
                            description: Text("Add a note — everyone in the family sees it."))
                        .frame(width: geometry.size.width, height: geometry.size.height)
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
                            boardSize: wall,
                            onMoved: { fraction in
                                Task { await coordinator.updateNote(id: note.noteID, x: fraction.x, y: fraction.y) }
                            },
                            onTap: {
                                editing = draft(for: note)
                            })
                    }
                }
                .frame(width: wall.width, height: wall.height, alignment: .topLeading)
                }
                .scrollBounceBehavior(.basedOnSize)
            }
            .background(BoardGround())
            .navigationTitle("Board")
            .navigationBarTitleDisplayMode(.inline)
            .photosPicker(
                isPresented: $showPhotoPicker,
                selection: $pickedPhoto,
                // A wall pins PICTURES: a video on a corkboard is a thing
                // to play (docs/protocol.md, "Board"). No `photoLibrary:`,
                // for the reason SettingsView gives — the out-of-process
                // picker needs no PhotoKit reference and no usage string.
                matching: .images)
            .onChange(of: pickedPhoto) { _, item in
                guard let item else { return }
                pinPicture(item, slot: Double(notes.count % 6))
            }
            .alert(
                "Couldn't pin that photo.",
                isPresented: Binding(
                    get: { pinFailure != nil },
                    set: { if !$0 { pinFailure = nil } })
            ) {
                Button("OK", role: .cancel) { pinFailure = nil }
            } message: {
                Text(pinFailure ?? "")
            }
            .overlay {
                if pinning {
                    // The upload can take a moment on a phone connection,
                    // and a wall that looked inert would be tapped again.
                    ProgressView().controlSize(.large)
                }
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        editing = newEventDraft()
                    } label: {
                        Image(systemName: "calendar.badge.plus")
                    }
                    .accessibilityLabel("Add an event")
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        editing = newListDraft()
                    } label: {
                        Image(systemName: "checklist")
                    }
                    .accessibilityLabel("Add a task list")
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        pickedPhoto = nil
                        showPhotoPicker = true
                    } label: {
                        Image(systemName: "photo.badge.plus")
                    }
                    .disabled(pinning)
                    .accessibilityLabel("Pin a photo")
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        editing = newTextDraft()
                    } label: {
                        Label("Add Note", systemImage: "plus")
                    }
                }
            }
            .sheet(item: $editing) { draft in
                editor(for: draft)
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

    private func save(
        draft: NoteDraft,
        text: String,
        color: String,
        size: NoteSize,
        font: NoteFont,
        event: EventEdit,
        lines: [DraftTaskLine]
    ) {
        editing = nil
        let isEvent = draft.kind == .event
        let isList = draft.kind == .tasks
        // Only a list's, and sent only when they DIFFER from what the note
        // holds: `items` is the author's field, and one sent unchanged
        // would make opening a list to read it an edit.
        let written = DraftTaskLine.written(lines)
        let held = draft.items.map { APIClient.TaskLineRequest(id: $0.id, text: $0.text) }
        Task {
            if let id = draft.noteID {
                await coordinator.updateNote(
                    id: id, text: text, color: color,
                    size: size.patchName(replacing: draft.storedSize),
                    font: font.patchName(replacing: draft.storedFont),
                    // Only an event's, and only when they moved: the server
                    // refuses all three on any other kind.
                    startsAt: isEvent ? event.startsAt : nil,
                    endsAt: isEvent ? .some(event.endsAt) : nil,
                    place: isEvent ? event.place : nil,
                    items: isList && written != held ? written : nil)
            } else {
                _ = await coordinator.addNote(
                    text: text, color: color, size: size.name, font: font.name,
                    x: draft.x, y: draft.y,
                    startsAt: isEvent ? event.startsAt : nil,
                    endsAt: isEvent ? event.endsAt : nil,
                    place: isEvent && !event.place.isEmpty ? event.place : nil,
                    // Empty is still a list — it is what makes the note
                    // one (docs/protocol.md, "Board").
                    items: isList ? written : nil)
            }
        }
    }

    /// The editor's view of one existing note.
    ///
    /// Its own function for the reason the two blank drafts are: a
    /// fourteen-argument literal nested inside a closure inside a `ForEach`
    /// inside a `GeometryReader` is more than the type-checker will do in
    /// reasonable time, and it says so rather than being slow.
    /// The note editor, built in a function of its own: a literal this
    /// long inside a view body is more than the type-checker will do in
    /// reasonable time, and it says so rather than being slow.
    private func editor(for draft: NoteDraft) -> NoteEditor {
        NoteEditor(
            draft: draft,
            canEdit: draft.noteID == nil || draft.authorID == currentUserID,
            authorName: displayName(for: draft.authorID),
            onSave: { text, color, size, font, event, lines in
                save(
                    draft: draft, text: text, color: color, size: size, font: font,
                    event: event, lines: lines)
            },
            onAnswer: { noteID, answer in
                Task { await coordinator.answerEvent(id: noteID, answer: answer) }
            },
            onTick: { noteID, itemID, done in
                Task { await coordinator.tickTask(noteID: noteID, itemID: itemID, done: done) }
            },
            onDelete: draft.noteID.map { id in { delete(id: id) } },
            currentUserID: currentUserID,
            onOpenChat: onOpenChat,
            mentionCandidates: mentionCandidates(matching:))
    }

    /// What the editor's strip offers for a half-typed name: the live
    /// roster, never the reader themself, never the blocked — the chat's
    /// own rule (docs/protocol.md, "Mentioning a member").
    private func mentionCandidates(matching query: String) -> [MentionDTO] {
        let roster = members
            .filter { !$0.hasLeft && !$0.accountDeleted }
            .map { MentionDTO(userID: $0.userID, name: $0.resolvedDisplayName) }
        return MemberMentions.candidates(
            in: roster, matching: query,
            excluding: coordinator.blockedUserIDs.union([currentUserID]))
    }

    private func draft(for note: NoteEntity) -> NoteDraft {
        NoteDraft(
            noteID: note.noteID,
            text: note.text,
            color: note.color,
            size: NoteSize(name: note.size),
            storedSize: note.size,
            font: NoteFont(name: note.font),
            storedFont: note.font,
            kind: NoteKind(name: note.kind),
            startsAt: note.startsAt,
            endsAt: note.endsAt,
            place: note.place ?? "",
            myAnswer: note.myAnswer(currentUserID),
            rsvps: note.rsvpList,
            mentions: note.mentionList,
            items: note.taskList,
            x: note.x,
            y: note.y,
            authorID: note.authorID)
    }

    /// A blank sticker, dropped near the top-left and offset a little each
    /// time so a burst of them does not stack into one illegible pile.
    ///
    /// Its own function for the same reason `newEventDraft` is.
    private func newTextDraft() -> NoteDraft {
        let slot = Double(notes.count % 6)
        return NoteDraft(
            noteID: nil,
            text: "",
            color: NoteColor.palette.randomElement() ?? "yellow",
            size: .medium,
            storedSize: nil,
            font: .plain,
            storedFont: nil,
            x: 0.12 + slot * 0.03,
            y: 0.10 + slot * 0.06,
            authorID: currentUserID)
    }

    /// A blank event, dropped where a new note lands.
    ///
    /// Its own function rather than an inline literal: fourteen defaulted
    /// arguments inside a `Button` closure inside a `ToolbarItem` is more
    /// than the type-checker will do in reasonable time, and it says so.
    private func newEventDraft() -> NoteDraft {
        let slot = Double(notes.count % 6)
        return NoteDraft(
            noteID: nil,
            text: "",
            color: "blue",
            size: .medium,
            storedSize: nil,
            font: .plain,
            storedFont: nil,
            kind: .event,
            // The next round hour: a family event is planned, not stamped
            // at the instant somebody tapped a button.
            startsAt: Date().nextRoundHour,
            x: 0.12 + slot * 0.03,
            y: 0.10 + slot * 0.06,
            authorID: currentUserID)
    }

    /// A blank task list, dropped where a new note lands.
    ///
    /// Its own function for the reason the other two blanks are: a
    /// literal this long inside a `Button` inside a `ToolbarItem` is more
    /// than the type-checker will do in reasonable time.
    private func newListDraft() -> NoteDraft {
        let slot = Double(notes.count % 6)
        return NoteDraft(
            noteID: nil,
            text: "",
            color: "green",
            size: .medium,
            storedSize: nil,
            font: .plain,
            storedFont: nil,
            kind: .tasks,
            x: 0.12 + slot * 0.03,
            y: 0.10 + slot * 0.06,
            authorID: currentUserID)
    }

    private func delete(id: Int64) {
        editing = nil
        Task { _ = await coordinator.deleteNote(id: id) }
    }

    /// Prepare, upload, pin — in that order, because the note may not exist
    /// until the picture does: the server claims the upload inside the same
    /// transaction that writes the note, and a note pointing at nothing is
    /// the one state this must never produce (docs/protocol.md, "Board").
    ///
    /// The picture is downscaled first, by the same MediaPrep a message
    /// uses: a wall tile is 220 points, and shipping 12 megapixels to draw
    /// it would cost the family's data for pixels nobody sees.
    private func pinPicture(_ item: PhotosPickerItem, slot: Double) {
        pinning = true
        Task {
            defer {
                pinning = false
                pickedPhoto = nil
            }
            // Reading it is one failure; pinning it is another, and each is
            // said as what it is — "couldn't read" for an upload that timed
            // out was a message about the wrong thing.
            let prepared: MediaPrep.Prepared
            do {
                guard let data = try await item.loadTransferable(type: Data.self) else {
                    pinFailure = String(localized: "Couldn't read that photo.")
                    return
                }
                prepared = try await MediaPrep.preparePhoto(from: data, limit: MediaPrep.sizeLimit)
            } catch {
                pinFailure = String(localized: "Couldn't read that photo.")
                return
            }
            defer { MediaPrep.discard(prepared) }
            pinFailure = await coordinator.pinPhoto(
                prepared,
                color: NoteColor.palette.randomElement() ?? "yellow",
                x: 0.12 + slot * 0.03,
                y: 0.10 + slot * 0.06)
        }
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

    /// A PHOTO WITH NO CAPTION IS THE BARE PICTURE (docs/protocol.md,
    /// "Board"): no paper behind it, no padding around it and no author
    /// line under it — a picture pinned to a wall. A caption brings the
    /// card back, because the words need paper to sit on.
    private var isBarePicture: Bool {
        !isHidden && NoteKind(name: note.kind) == .photo && note.text.isEmpty
    }

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
            // A pinned picture fills the sticker, with the caption under it
            // — and NOTHING while the note is hidden by a block: the
            // picture is content, exactly as the text is (protocol.md,
            // "Board").
            if !isHidden, NoteKind(name: note.kind) == .photo, let attachmentID = note.attachmentID {
                NotePicture(
                    attachmentID: attachmentID,
                    height: NotePicture.height(cardHeight: side, hasCaption: !note.text.isEmpty))
            }
            // An event says WHEN before it says what: the date is the
            // reason it is on the wall (protocol.md, "Board").
            if !isHidden, NoteKind(name: note.kind) == .event, let starts = note.startsAt {
                NoteEventBlock(
                    starts: starts,
                    ends: note.endsAt,
                    place: note.place,
                    going: note.answerCount(RsvpAnswer.going.name),
                    maybe: note.answerCount(RsvpAnswer.maybe.name))
            }
            if !isBarePicture {
            // The names the note says are drawn as names — bold, in the
            // note's own ink, and NOT doors here: a sticker's face is a
            // drag handle, and the door is in the note when it is opened
            // (docs/protocol.md, "Board").
            (isHidden
                ? Text("Hidden — blocked member")
                : Text(MemberMentions.noteText(
                    note.text, mentions: note.mentionList, linking: false)))
                // The hand the author chose (docs/protocol.md, "Board").
                // A hidden note keeps it, like its colour and its tilt:
                // nothing about the shape of a note is the blocked
                // member's content.
                .font(NoteFont(name: note.font).font(for: size))
                .foregroundStyle(.black.opacity(isHidden ? 0.45 : 0.85))
                .italic(isHidden)
                // The text FITS the sticker (docs/protocol.md, "Board"):
                // the type scales down from the size's own until the whole
                // note is inside it, and only past the floor is anything
                // cut. `lineLimit` is a backstop for one unbroken word, not
                // the layout rule it used to be.
                .lineLimit(size.fittedLineLimit)
                .minimumScaleFactor(size.minimumTextScale)
            Spacer(minLength: 0)
            }
            // No author line at all while hidden — not an empty one, which
            // would still say a note came from somebody. And none on a
            // bare picture either: there is no paper under it to write on,
            // and the name is in the note when it is opened.
            if !isHidden && !isBarePicture {
                Text(authorName)
                    .font(.caption2)
                    .foregroundStyle(.black.opacity(0.5))
            }
        }
        .padding(isBarePicture ? 0 : 10)
        .frame(width: side, height: side, alignment: .topLeading)
        .background(
            isBarePicture ? Color.clear : NoteColor.swiftUI(note.color),
            in: RoundedRectangle(cornerRadius: isBarePicture ? 4 : 10))
        .shadow(color: .black.opacity(drag == .zero ? 0.12 : 0.25), radius: drag == .zero ? 3 : 10, y: 2)
        // THE PIN, over the card's top edge: an overlay, so it takes none
        // of the room the words need — the wall's promise is that the text
        // fits the note (docs/protocol.md, "Board").
        .overlay(alignment: .top) { NotePin().offset(y: -4) }
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
    let onSave: (String, String, NoteSize, NoteFont, EventEdit, [DraftTaskLine]) -> Void
    /// Answering is its own act — any member may, so it does not go
    /// through `onSave`, which is the author's.
    var onAnswer: (Int64, String?) -> Void = { _, _ in }
    /// Ticking a line is its own act too, and for the same reason: any
    /// member may, so it does not go through `onSave` either
    /// (docs/protocol.md, "Board").
    var onTick: (Int64, Int64, Bool) -> Void = { _, _, _ in }
    let onDelete: (() -> Void)?
    /// Who is reading, and where a name goes — see the reader's view
    /// below (docs/protocol.md, "Board").
    var currentUserID: Int64 = -1
    var onOpenChat: ((Int64) -> Void)?
    /// The members a half-typed `@` could mean, and what picking one
    /// writes into the text.
    var mentionCandidates: (String) -> [MentionDTO] = { _ in [] }

    @Environment(\.dismiss) private var dismiss

    /// The names this reader cannot open a chat with: whoever the STRIP
    /// would offer for a bare `@` is whoever a name may open.
    private var closedNames: Set<Int64> {
        MemberMentions.closedNames(in: draft.mentions, openTo: mentionCandidates(""))
    }

    @State private var text: String
    @State private var color: String
    @State private var size: NoteSize
    @State private var font: NoteFont
    /// An event's own three, edited here by its author.
    @State private var startsAt: Date
    @State private var hasEnd: Bool
    @State private var endsAt: Date
    @State private var place: String
    @State private var confirmDelete = false
    /// A task list's lines as the author is writing them.
    @State private var lines: [DraftTaskLine]
    /// Ticks on their way: the line and the state being sent, so a box
    /// answers the tap at once and goes back to the note's own truth when
    /// the answer — or the refusal — lands.
    @State private var ticking: [Int64: Bool] = [:]

    init(
        draft: NoteDraft,
        canEdit: Bool,
        authorName: String,
        onSave: @escaping (String, String, NoteSize, NoteFont, EventEdit, [DraftTaskLine]) -> Void,
        onAnswer: @escaping (Int64, String?) -> Void = { _, _ in },
        onTick: @escaping (Int64, Int64, Bool) -> Void = { _, _, _ in },
        onDelete: (() -> Void)?,
        currentUserID: Int64 = -1,
        onOpenChat: ((Int64) -> Void)? = nil,
        mentionCandidates: @escaping (String) -> [MentionDTO] = { _ in [] }
    ) {
        self.draft = draft
        self.canEdit = canEdit
        self.authorName = authorName
        self.onSave = onSave
        self.onAnswer = onAnswer
        self.onTick = onTick
        self.onDelete = onDelete
        self.currentUserID = currentUserID
        self.onOpenChat = onOpenChat
        self.mentionCandidates = mentionCandidates
        // A new list opens with one empty line, so the first thing to do
        // is one tap away rather than two.
        _lines = State(initialValue: draft.kind == .tasks && draft.items.isEmpty && draft.noteID == nil
            ? [DraftTaskLine()]
            : draft.items.map { DraftTaskLine(itemID: $0.id, text: $0.text) })
        _text = State(initialValue: draft.text)
        _color = State(initialValue: draft.color)
        _size = State(initialValue: draft.size)
        _font = State(initialValue: draft.font)
        _startsAt = State(initialValue: draft.startsAt ?? Date())
        _hasEnd = State(initialValue: draft.endsAt != nil)
        // An hour after the start is the shape most family things take, and
        // it is only a starting point for the picker.
        _endsAt = State(
            initialValue: draft.endsAt ?? (draft.startsAt ?? Date()).addingTimeInterval(3600))
        _place = State(initialValue: draft.place)
    }

    private var isEvent: Bool { draft.kind == .event }
    private var isList: Bool { draft.kind == .tasks }

    private var navigationTitle: LocalizedStringKey {
        switch (draft.kind, draft.noteID == nil) {
        case (.tasks, true): "New List"
        case (.tasks, false): "List"
        case (.event, true): "New Event"
        case (.event, false): "Event"
        case (_, true): "New Note"
        case (_, false): "Note"
        }
    }

    /// What a box draws: the tap's own answer while one is in flight, and
    /// the note's own truth otherwise.
    private func isDone(_ itemID: Int64?) -> Bool {
        guard let itemID else { return false }
        if let sending = ticking[itemID] { return sending }
        return draft.items.first { $0.id == itemID }?.done ?? false
    }

    /// Tick or untick, and show it at once. One request per line at a
    /// time: a second tap while the first is in flight is the tap that
    /// would undo it.
    private func tick(_ itemID: Int64, to done: Bool) {
        guard let noteID = draft.noteID, ticking[itemID] == nil else { return }
        ticking[itemID] = done
        onTick(noteID, itemID, done)
    }

    /// This reader's answer, kept locally so the picker moves at once —
    /// the note itself comes back through the board feed.
    @State private var answeredOverride: String??

    private var answered: String? {
        if case let .some(value) = answeredOverride { return value }
        return draft.myAnswer
    }

    private func answer(_ choice: RsvpAnswer?, on noteID: Int64) {
        answeredOverride = .some(choice?.name)
        onAnswer(noteID, choice?.name)
    }

    private var guestLine: String {
        let going = draft.rsvps.count { $0.answer == RsvpAnswer.going.name }
        let maybe = draft.rsvps.count { $0.answer == RsvpAnswer.maybe.name }
        let no = draft.rsvps.count { $0.answer == RsvpAnswer.no.name }
        return String(localized: "\(going) going · \(maybe) maybe · \(no) can't")
    }

    /// The lines of a task list: a box everybody may tap, and — for the
    /// author — the words, the remove and the add.
    ///
    /// Its own property for the reason the drafts are functions: a Form
    /// section this long inside the body is more than the type-checker
    /// will do in reasonable time.
    @ViewBuilder
    private var taskSection: some View {
        Section("Things to do") {
            if lines.isEmpty {
                Text("Nothing on this list yet.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ForEach($lines) { $line in
                NoteTaskRow(
                    itemID: line.itemID,
                    done: isDone(line.itemID),
                    canEdit: canEdit,
                    onTick: { done in
                        if let itemID = line.itemID { tick(itemID, to: done) }
                    },
                    onRemove: canEdit
                        ? { lines.removeAll { $0.id == line.id } }
                        : nil,
                    text: $line.text)
                    // The cap where the typing is, as the title has it: a
                    // line the server would refuse never becomes a save
                    // that fails (docs/protocol.md, "Board").
                    .onChange(of: line.text) { _, new in
                        let capped = NoteText.capped(new, to: NoteText.maxTaskItemLength)
                        if capped != new { line.text = capped }
                    }
            }
            if canEdit {
                Button {
                    lines.append(DraftTaskLine())
                } label: {
                    Label("Add a thing", systemImage: "plus.circle")
                }
                // Held to the server's ceiling here, where somebody can
                // see why: a twenty-first line typed and then refused is
                // a save that fails for a reason nobody was shown.
                .disabled(lines.count >= NoteText.maxTaskItems)
            }
            if !draft.items.isEmpty {
                Text("\(draft.items.count { isDone($0.id) }) of \(draft.items.count) done")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if canEdit {
                        TextField("Note", text: $text, axis: .vertical)
                            .lineLimit(3...8)
                            // The cap lives where the typing is: a note the
                            // server would refuse never becomes a failed
                            // save (docs/protocol.md, "Board").
                            .onChange(of: text) { _, new in
                                let capped = NoteText.capped(new)
                                if capped != new { text = capped }
                            }
                        // The names a half-typed `@` could mean
                        // (docs/protocol.md, "Board"): the chat's own strip,
                        // under the words being written.
                        if let query = MemberMentions.query(in: text) {
                            let offered = mentionCandidates(query)
                            if !offered.isEmpty {
                                MentionSuggestions(candidates: offered) { name in
                                    text = MemberMentions.accept(draft: text, name: name)
                                }
                            }
                        }
                        if NoteText.shouldShowCounter(text) {
                            Text("\(NoteText.remaining(text)) characters left")
                                .font(.caption)
                                .foregroundStyle(NoteText.remaining(text) == 0 ? .red : .secondary)
                        }
                    } else {
                        // A name in an OPEN note is a door (docs/protocol.md,
                        // "Board") — but only where there is somebody to
                        // open it with, which the STRIP already answers:
                        // whoever it would offer for a bare `@` is whoever
                        // a name may open. So the reader's own name, a
                        // member they blocked, and one who has left or
                        // deleted their account are highlighted like any
                        // other and simply do not open.
                        Text(MemberMentions.noteText(
                            draft.text,
                            mentions: draft.mentions,
                            linking: onOpenChat != nil,
                            excluding: closedNames))
                        .environment(\.openURL, OpenURLAction { url in
                            guard let userID = MemberMentions.userID(from: url) else {
                                return .systemAction
                            }
                            dismiss()
                            onOpenChat?(userID)
                            return .handled
                        })
                        Text("Written by \(authorName)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                // WHEN and WHERE, above the look: they are why the note is
                // on the wall (protocol.md, "Board").
                if isEvent, canEdit {
                    Section("When") {
                        DatePicker("Starts", selection: $startsAt)
                        Toggle("Has an end", isOn: $hasEnd.animation())
                        if hasEnd {
                            DatePicker("Ends", selection: $endsAt, in: startsAt...)
                        }
                    }
                    Section("Where") {
                        TextField("Place", text: $place)
                            // Scalars, as the server counts: a place that
                            // looked under 200 graphemes could be refused.
                            .onChange(of: place) { _, new in
                                let capped = NoteText.capped(new, to: NoteText.maxPlaceLength)
                                if capped != new { place = capped }
                            }
                    }
                } else if isEvent, let starts = draft.startsAt {
                    // Somebody else's event: read it, and say whether you
                    // are coming — ANSWERING IS NOT AUTHORSHIP.
                    Section("When") {
                        Text(EventFormat.when(starts: starts, ends: draft.endsAt))
                        if !draft.place.isEmpty {
                            Text(draft.place).foregroundStyle(.secondary)
                        }
                    }
                }
                // THE LIST. One section for the author and for everybody
                // else, because the boxes are everybody's: what `canEdit`
                // adds is the words beside each box, the remove and the
                // add (docs/protocol.md, "Board").
                if isList {
                    taskSection
                }
                if isEvent, let noteID = draft.noteID {
                    Section("Are you coming?") {
                        Picker("Are you coming?", selection: Binding(
                            get: { RsvpAnswer(name: answered) },
                            set: { answer($0, on: noteID) })
                        ) {
                            Text("No answer").tag(RsvpAnswer?.none)
                            ForEach(RsvpAnswer.allCases) { choice in
                                Text(choice.title).tag(RsvpAnswer?.some(choice))
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        if !draft.rsvps.isEmpty {
                            Text(guestLine)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
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
                    // The hand, with text, colour and size: all four are
                    // the author's (docs/protocol.md, "Board").
                    Section("Font") {
                        Picker("Font", selection: $font) {
                            ForEach(NoteFont.allCases) { face in
                                Text(face.title)
                                    .font(Font.system(.body, design: face.design))
                                    .tag(face)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        // The consequence, in front of the author: the same
                        // sticker the wall will draw, with the type already
                        // fitted (docs/protocol.md, "Board").
                        HStack {
                            Spacer(minLength: 0)
                            NotePreview(
                                text: text.isEmpty ? String(localized: "Your note") : text,
                                color: color,
                                size: size,
                                font: font)
                            Spacer(minLength: 0)
                        }
                        .padding(.vertical, 4)
                        .animation(.spring(duration: 0.2), value: size)
                        .animation(.spring(duration: 0.2), value: font)
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
            .navigationTitle(navigationTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
                if canEdit {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") {
                            onSave(
                                text, color, size, font,
                                EventEdit(
                                    startsAt: isEvent ? startsAt : nil,
                                    endsAt: isEvent && hasEnd ? endsAt : nil,
                                    place: isEvent ? place : ""),
                                lines)
                        }
                            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
    }
}


#endif
