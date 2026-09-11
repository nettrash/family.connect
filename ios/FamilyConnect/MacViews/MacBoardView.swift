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
    /// A note or an event: the one sheet writes both.
    @State private var composingKind = NoteKind.text
    @State private var draftText = ""
    @State private var draftColor = NoteColor.palette.first ?? "yellow"
    @State private var draftSize = NoteSize.medium
    @State private var draftFont = NoteFont.plain
    @State private var draftEvent = MacEventFields()
    /// A new task list's lines (docs/protocol.md, "Board").
    @State private var draftLines: [DraftTaskLine] = []
    /// Pinning a picture: the upload it turns into, and what went wrong.
    @State private var pinning = false
    @State private var pinFailure: String?

    var body: some View {
        GeometryReader { geometry in
            // THE WALL SCROLLS: it is taller than the window
            // (docs/protocol.md, "Board"), so a family can pin something
            // without taking something down. The fractions are read
            // against the WALL, which is why the notes are given its size
            // and not the window's.
            let wall = BoardWall.size(visible: geometry.size)
            ScrollView(.vertical) {
                ZStack(alignment: .topLeading) {
                    BoardGround()
                    ForEach(notes) { note in
                        sticker(for: note, board: wall)
                    }
                    if notes.isEmpty {
                        ContentUnavailableView(
                            "The board is empty",
                            systemImage: "square.grid.2x2",
                            description: Text("Add a note — everyone in the family sees it."))
                        .frame(width: geometry.size.width, height: geometry.size.height)
                    }
                }
                .frame(width: wall.width, height: wall.height, alignment: .topLeading)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .frame(minWidth: 480, minHeight: 360)
        .navigationTitle("Board")
        .toolbar {
            ToolbarItem {
                Button {
                    startComposing(.text)
                } label: {
                    Label("Add Note", systemImage: "plus")
                }
                .keyboardShortcut("n", modifiers: .command)
                .help("Add a note")
            }
            // The Mac could not pin an event at all — only answer one from a
            // context menu, on a card that did not even say when it was
            // (issue #69). The phone's button, the phone's fields.
            ToolbarItem {
                Button {
                    startComposing(.event)
                } label: {
                    Label("Add an event", systemImage: "calendar.badge.plus")
                }
                .help("Add an event")
            }
            ToolbarItem {
                Button {
                    startComposing(.tasks)
                } label: {
                    Label("Add List", systemImage: "checklist")
                }
                .help("Add a task list")
            }
            ToolbarItem {
                Button {
                    pinPicture()
                } label: {
                    Label("Pin a Photo", systemImage: "photo.badge.plus")
                }
                .disabled(pinning)
                .help("Pin a photo")
            }
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
                ProgressView().controlSize(.large)
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
            MacNoteEditor(
                text: $draftText, color: $draftColor, size: $draftSize, font: $draftFont,
                event: $draftEvent,
                kind: composingKind,
                title: newTitle,
                mentionCandidates: mentionCandidates(matching:),
                lines: $draftLines
            ) {
                let isEvent = composingKind == .event
                let isList = composingKind == .tasks
                let event = draftEvent
                let lines = draftLines
                Task {
                    // Dropped near the middle with a little scatter, so a
                    // run of new notes does not stack into one pile.
                    _ = await coordinator.addNote(
                        text: draftText,
                        color: draftColor,
                        size: draftSize.name,
                        font: draftFont.name,
                        x: Double.random(in: 0.25...0.65),
                        y: Double.random(in: 0.25...0.65),
                        startsAt: isEvent ? event.startsAt : nil,
                        endsAt: isEvent && event.hasEnd ? event.endsAt : nil,
                        place: isEvent && !event.trimmedPlace.isEmpty ? event.trimmedPlace : nil,
                        // Empty is still a list — it is what makes the
                        // note one (docs/protocol.md, "Board").
                        items: isList ? DraftTaskLine.written(lines) : nil)
                }
            }
        }
        .sheet(item: $editing) { note in
            MacNoteEditorForExisting(note: note)
        }
    }

    private var newTitle: LocalizedStringKey {
        switch composingKind {
        case .event: "New Event"
        case .tasks: "New List"
        default: "New Note"
        }
    }

    /// What the marks WOULD be if this wall counted as shown — an
    /// Equatable value, so `onChange` fires on a note arriving, on one
    /// being rewritten, and on nothing else. A drag changes no part of it.
    private var boardMark: BoardBadge.Marks {
        BoardBadge.marksAfterShowing(notes: notes, marks: .zero)
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
            excluding: coordinator.blockedUserIDs.union([coordinator.currentUserID]))
    }

    /// One sticker. Its own function for the reason BoardView's drafts are:
    /// a literal this long inside a `ForEach` inside a `GeometryReader` is
    /// more than the type-checker will do in reasonable time, and it says so
    /// rather than being slow.
    private func sticker(for note: NoteEntity, board: CGSize) -> MacNoteView {
        MacNoteView(
            note: note,
            board: board,
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
            },
            myAnswer: note.myAnswer(coordinator.currentUserID),
            onAnswer: { answer in
                Task { await coordinator.answerEvent(id: note.noteID, answer: answer) }
            },
            onTick: { itemID, done in
                Task {
                    await coordinator.tickTask(
                        noteID: note.noteID, itemID: itemID, done: done)
                }
            },
            names: displayName(for:),
            canDraw: AppSettings.assistantImages,
            onDrawBackdrop: {
                Task { await coordinator.drawBackdrop(noteID: note.noteID) }
            })
    }

    /// A blank note — or event — to write. An event starts on the next
    /// round hour and is blue, as on the phone; a note is any colour.
    private func startComposing(_ kind: NoteKind) {
        composingKind = kind
        draftText = ""
        draftColor = switch kind {
        case .event: "blue"
        case .tasks: "green"
        default: NoteColor.palette.randomElement() ?? "yellow"
        }
        draftSize = .medium
        draftFont = .plain
        draftEvent = MacEventFields()
        // A new list opens with one empty line, so the first thing to do
        // is one tap away rather than two.
        draftLines = kind == .tasks ? [DraftTaskLine()] : []
        composing = true
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
fileprivate struct MacNoteView: View {
    @State private var confirmDelete = false
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
    /// What this reader has answered, if it is an event.
    var myAnswer: String?
    /// Say whether you are coming — ANY member may, so it sits outside
    /// every author gate (docs/protocol.md, "Board"). nil retracts.
    var onAnswer: (String?) -> Void = { _ in }
    /// Tick a line off the list — ANY member may, for the same reason, and
    /// on this platform the MENU is where they do it: a reader cannot open
    /// somebody else's note here, and ticking must be reachable by every
    /// member on every client (docs/protocol.md, "Board").
    var onTick: (Int64, Bool) -> Void = { _, _ in }
    /// Every name this family has, for naming who is coming
    /// (docs/protocol.md, "Board").
    var names: (Int64) -> String = { _ in "" }
    /// Whether this SERVER can draw at all (`assistant.images`).
    var canDraw: Bool = false
    /// Ask the assistant for a backdrop — the author's.
    var onDrawBackdrop: () -> Void = {}

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

    /// A PHOTO WITH NO CAPTION IS THE BARE PICTURE (docs/protocol.md,
    /// "Board") — the same rule the phone follows: no paper behind it, no
    /// padding around it, no author line under it.
    private var isBarePicture: Bool {
        !isHidden && NoteKind(name: note.kind) == .photo && note.text.isEmpty
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
            // A pinned picture fills the card, with the caption under it —
            // and nothing while hidden by a block: the picture is content,
            // exactly as the text is (protocol.md, "Board").
            if !isHidden, NoteKind(name: note.kind) == .photo, let attachmentID = note.attachmentID {
                NotePicture(
                    attachmentID: attachmentID,
                    height: NotePicture.height(cardHeight: size.height, hasCaption: !note.text.isEmpty))
            }
            // An event says WHEN before it says what — the date is the reason
            // it is on the wall — and who is coming. The Mac drew an event as
            // its bare title (issue #69).
            if !isHidden, NoteKind(name: note.kind) == .event, let starts = note.startsAt {
                NoteEventBlock(
                    starts: starts,
                    ends: note.endsAt,
                    place: note.place,
                    going: note.answerCount(RsvpAnswer.going.name),
                    maybe: note.answerCount(RsvpAnswer.maybe.name))
            }
            // A LIST says what is on it, under its title: the first
            // lines with their state, and then how many are left. No click
            // here — the tick is in the menu on this platform
            // (docs/protocol.md, "Board").
            if !isHidden, NoteKind(name: note.kind) == .tasks {
                NoteTaskBlock(items: note.taskList)
            }
            if !isBarePicture {
            // The names, bold and in the note's own ink — and not doors on
            // the wall, for the reason BoardView gives (docs/protocol.md,
            // "Board").
            (isHidden
                ? Text("Hidden — blocked member")
                : Text(MemberMentions.noteText(
                    note.text, mentions: note.mentionList, linking: false)))
                // The hand the author chose (docs/protocol.md, "Board").
                .font(NoteFont(name: note.font).font(for: noteSize))
                // Forced ink, matching BoardView: the pastels are fixed
                // light colors in both appearances, so .primary’s dark-mode
                // white was unreadable on them.
                .foregroundStyle(.black.opacity(isHidden ? 0.45 : 0.85))
                .italic(isHidden)
                // The text FITS the sticker (docs/protocol.md, "Board") —
                // the same rule and the same floor as the phone.
                .lineLimit(noteSize.fittedLineLimit)
                .minimumScaleFactor(noteSize.minimumTextScale)
            Spacer(minLength: 0)
            }
            // No author line at all while hidden — nor on a bare picture,
            // which has no paper under it to write one on.
            if !isHidden && !isBarePicture {
                Text(authorName)
                    .font(.caption2)
                    .foregroundStyle(.black.opacity(0.5))
            }
        }
        .padding(isBarePicture ? 0 : 10)
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        .background(
            isBarePicture ? Color.clear : NoteColor.swiftUI(note.color),
            in: RoundedRectangle(cornerRadius: isBarePicture ? 4 : 8))
        // Lifted off the wall while it is in hand — the same cue the phone
        // gives, and the only feedback a cursor drag has.
        .shadow(color: .black.opacity(isDragging ? 0.28 : 0.12),
                radius: isDragging ? 10 : 3, y: 2)
        // The pin, over the card's top edge — an overlay, so the words
        // keep all their room (docs/protocol.md, "Board").
        .overlay(alignment: .top) { NotePin().offset(y: -4) }
        .scaleEffect(isDragging ? 1.04 : 1)
        .animation(.easeOut(duration: 0.12), value: isDragging)
        // Asks first, as the phone does: one menu click used to take a
        // note off the whole family's wall.
        .confirmationDialog("Delete this note?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive, action: onDelete)
        }
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
                Button("Delete", role: .destructive) { confirmDelete = true }
            } else {
                // Anyone may MOVE a note; only its author may change it.
                Text("Written by someone else")
            }
            // TICKING IS NOT AUTHORSHIP either, and this is the one
            // client where the menu is the only way to it: a reader never
            // opens somebody else's note here (protocol.md, "Board").
            if !isHidden, NoteKind(name: note.kind) == .tasks, !note.taskList.isEmpty {
                Divider()
                Menu("Things to do") {
                    ForEach(note.taskList, id: \.id) { item in
                        Toggle(item.text, isOn: Binding(
                            get: { item.done },
                            set: { onTick(item.id, $0) }))
                    }
                }
            }
            // WHO IS COMING, by name — and on this client the menu IS
            // the opened note, so this is where the names belong
            // (protocol.md, "Board"). Plain rows: it is news, not an
            // action.
            if !isHidden, NoteKind(name: note.kind) == .event {
                Divider()
                if guestGroups.isEmpty {
                    Text("Nobody has answered yet.")
                } else {
                    ForEach(guestGroups, id: \.0) { group in
                        Text(verbatim: "\(group.0.plainTitle): \(group.1)")
                    }
                }
                // A copy for this reader's own calendar — anybody's — and
                // the picture behind it, which is the author's.
                Button("Add to Calendar") { addToCalendar() }
                if isMine, canDraw {
                    Button(
                        note.attachmentID == nil
                            ? "Draw a backdrop"
                            : "Draw another backdrop",
                        action: onDrawBackdrop)
                }
            }
            // ANSWERING IS NOT AUTHORSHIP: outside the isMine branch on
            // purpose, and offered on a hidden note no more than its text
            // is (protocol.md, "Board").
            if !isHidden, NoteKind(name: note.kind) == .event {
                Divider()
                // Checkmarked, like the Size menu: the answer already given
                // reads as a state, where plain buttons said nothing about it.
                Menu("Are you coming?") {
                    ForEach(RsvpAnswer.allCases) { choice in
                        Toggle(choice.title, isOn: Binding(
                            get: { RsvpAnswer(name: myAnswer) == choice },
                            set: { on in onAnswer(on ? choice.name : nil) }))
                    }
                    Divider()
                    Toggle("No answer", isOn: Binding(
                        get: { RsvpAnswer(name: myAnswer) == nil },
                        set: { on in if on { onAnswer(nil) } }))
                }
            }
        }
    }
}

extension MacNoteView {
    /// Who is coming, grouped by answer and named — the sticker's counts
    /// with the people put back (docs/protocol.md, "Board").
    var guestGroups: [(RsvpAnswer, String)] {
        RsvpAnswer.allCases.compactMap { choice in
            let named = note.rsvpList
                .filter { $0.answer == choice.name }
                .map { names($0.userID) }
                .filter { !$0.isEmpty }
            return named.isEmpty ? nil : (choice, named.joined(separator: ", "))
        }
    }

    /// Write the `.ics` and let the Mac open it — Calendar's own import
    /// dialog, which costs no permission and no entitlement
    /// (EventCalendar).
    func addToCalendar() {
        let title = note.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let ics = EventCalendar.ics(
            noteID: note.noteID,
            title: title,
            startsAt: note.startsAt,
            endsAt: note.endsAt,
            place: note.place),
            let file = EventCalendar.file(named: title, ics: ics)
        else { return }
        NSWorkspace.shared.open(file)
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

/// What an event adds to a note: when it starts, whether and when it ends,
/// and where. An hour is the shape most family things take, and only a
/// starting point for the pickers.
struct MacEventFields {
    var startsAt: Date = Date().nextRoundHour
    var hasEnd = false
    var endsAt: Date = Date().nextRoundHour.addingTimeInterval(3600)
    var place = ""

    /// Trimmed, as the server stores it.
    var trimmedPlace: String { place.trimmingCharacters(in: .whitespacesAndNewlines) }
}

/// Compose a new note — or event — or rewrite one.
private struct MacNoteEditor: View {
    @Binding var text: String
    @Binding var color: String
    @Binding var size: NoteSize
    @Binding var font: NoteFont
    @Binding var event: MacEventFields
    /// What is being written. An event adds its when and where; a photo's
    /// caption may be left empty — the picture is the note, and a caption
    /// the Mac insisted on made a caption-less photo impossible to recolour
    /// or resize at all.
    let kind: NoteKind
    /// A key, not a String: `Text(title)` then goes through the catalog
    /// ("New Note" / "Edit Note") instead of shipping English verbatim.
    let title: LocalizedStringKey
    /// The members a half-typed `@` could mean (docs/protocol.md,
    /// "Board"). Empty where nobody is offered, which is what a board with
    /// no roster loaded yet has.
    var mentionCandidates: (String) -> [MentionDTO] = { _ in [] }
    /// A task list's lines as the author is writing them (docs/protocol.md,
    /// "Board"). Empty on every other kind.
    @Binding var lines: [DraftTaskLine]
    /// What a line's box draws, and what a tap on it asks for. Ticking is
    /// ANY member's act, so it does not go through `onSave`; on a list
    /// nobody has saved yet there is nothing to tick and the boxes are
    /// disabled.
    var isDone: (Int64) -> Bool = { _ in false }
    var onTick: (Int64, Bool) -> Void = { _, _ in }
    let onSave: () -> Void

    private var canSave: Bool {
        kind == .photo || !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// What the preview says before anything is typed.
    private var blankPreview: String.LocalizationValue {
        switch kind {
        case .tasks: "Your list"
        case .event: "Your event"
        default: "Your note"
        }
    }

    /// The lines the author is writing, each with the box that says
    /// whether it is done.
    ///
    /// Its own property for the reason the phone's is: a block this long
    /// inside the body is more than the type-checker will do in reasonable
    /// time.
    @ViewBuilder
    private var taskLines: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Things to do").font(.subheadline)
            ForEach($lines) { $line in
                NoteTaskRow(
                    itemID: line.itemID,
                    done: line.itemID.map(isDone) ?? false,
                    canEdit: true,
                    onTick: { done in
                        if let itemID = line.itemID { onTick(itemID, done) }
                    },
                    onRemove: { lines.removeAll { $0.id == line.id } },
                    text: $line.text)
                    // The cap where the typing is, as the title has it.
                    .onChange(of: line.text) { _, new in
                        let capped = NoteText.capped(new, to: NoteText.maxTaskItemLength)
                        if capped != new { line.text = capped }
                    }
            }
            Button {
                lines.append(DraftTaskLine())
            } label: {
                Label("Add a thing", systemImage: "plus.circle")
            }
            .disabled(lines.count >= NoteText.maxTaskItems)
        }
        .frame(width: 320, alignment: .leading)
    }

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            TextEditor(text: $text)
                .frame(width: 320, height: 120)
                .border(.separator)
                // The cap where the typing is, as on the phone
                // (docs/protocol.md, "Board").
                .onChange(of: text) { _, new in
                    let capped = NoteText.capped(new)
                    if capped != new { text = capped }
                }
            // The names a half-typed `@` could mean — the chat's own strip,
            // under the words (docs/protocol.md, "Board").
            if let query = MemberMentions.query(in: text) {
                let offered = mentionCandidates(query)
                if !offered.isEmpty {
                    MentionSuggestions(candidates: offered) { name in
                        text = MemberMentions.accept(draft: text, name: name)
                    }
                    .frame(width: 320)
                }
            }
            if NoteText.shouldShowCounter(text) {
                Text("\(NoteText.remaining(text)) characters left")
                    .font(.caption)
                    .foregroundStyle(NoteText.remaining(text) == 0 ? .red : .secondary)
            }
            // THE LINES, above the look: they are what the note says
            // (docs/protocol.md, "Board").
            if kind == .tasks {
                taskLines
            }
            // WHEN and WHERE, above the look: they are why the note is on
            // the wall (docs/protocol.md, "Board").
            if kind == .event {
                DatePicker("Starts", selection: $event.startsAt)
                    // A start moved past the end takes the end with it.
                    .onChange(of: event.startsAt) { _, starts in
                        if event.endsAt < starts { event.endsAt = starts.addingTimeInterval(3600) }
                    }
                Toggle("Has an end", isOn: $event.hasEnd)
                if event.hasEnd {
                    DatePicker("Ends", selection: $event.endsAt, in: event.startsAt...)
                }
                TextField("Place", text: $event.place)
                    // Scalars, as the server counts them.
                    .onChange(of: event.place) { _, new in
                        let capped = NoteText.capped(new, to: NoteText.maxPlaceLength)
                        if capped != new { event.place = capped }
                    }
            }
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
            // The hand, with text, colour and size: all four are the
            // author's (docs/protocol.md, "Board").
            Picker("Font", selection: $font) {
                ForEach(NoteFont.allCases) { face in
                    Text(face.title)
                        .font(Font.system(.body, design: face.design))
                        .tag(face)
                }
            }
            .pickerStyle(.segmented)
            // The sticker as the wall will draw it, type already fitted.
            HStack {
                Spacer(minLength: 0)
                NotePreview(
                    text: text.isEmpty ? String(localized: blankPreview) : text,
                    color: color,
                    size: size,
                    font: font)
                Spacer(minLength: 0)
            }
            .animation(.easeOut(duration: 0.15), value: size)
            .animation(.easeOut(duration: 0.15), value: font)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    onSave()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!canSave)
            }
        }
        .padding(16)
    }
}

extension MacBoardView {
    /// Pick, prepare, pin. The pin itself — upload, preview, note, in that
    /// order — is the coordinator's, shared with the phone (issue #69); the
    /// pick is the Mac's own open panel, which reaches the file system and,
    /// through its Media sidebar, the Photos library as well.
    fileprivate func pinPicture() {
        guard let url = MacFilePicker.pickPhotoToPin() else { return }
        pinning = true
        Task {
            defer { pinning = false }
            let prepared: MediaPrep.Prepared
            do {
                prepared = try await MediaPrep.prepare(fileAt: url, limit: MediaPrep.sizeLimit)
            } catch {
                pinFailure = String(localized: "Couldn't read that photo.")
                return
            }
            defer { MediaPrep.discard(prepared) }
            // An animated image prepares as a file: a wall pins pictures.
            guard prepared.kind == AttachmentDTO.Kind.photo else {
                pinFailure = String(localized: "The board pins photos only.")
                return
            }
            pinFailure = await coordinator.pinPhoto(
                prepared,
                color: NoteColor.palette.randomElement() ?? "yellow",
                x: 0.35 + Double.random(in: -0.05...0.05),
                y: 0.30 + Double.random(in: -0.05...0.05))
        }
    }
}

/// Rewrite one that already exists — author only, which the caller gates.
private struct MacNoteEditorForExisting: View {
    let note: NoteEntity

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @Query private var members: [MemberEntity]
    @State private var text: String = ""
    @State private var color: String = "yellow"
    @State private var size: NoteSize = .medium
    @State private var font: NoteFont = .plain
    @State private var event = MacEventFields()
    @State private var lines: [DraftTaskLine] = []
    /// Ticks on their way: the line and the state being sent, so a box
    /// answers the click at once and goes back to the note's own truth
    /// when the answer — or the refusal — lands.
    @State private var ticking: [Int64: Bool] = [:]

    private var kind: NoteKind { NoteKind(name: note.kind) }

    private var editTitle: LocalizedStringKey {
        switch kind {
        case .event: "Edit Event"
        case .tasks: "Edit List"
        default: "Edit Note"
        }
    }

    /// The same strip the new-note editor gets (docs/protocol.md, "Board").
    private func mentionCandidates(matching query: String) -> [MentionDTO] {
        let roster = members
            .filter { !$0.hasLeft && !$0.accountDeleted }
            .map { MentionDTO(userID: $0.userID, name: $0.resolvedDisplayName) }
        return MemberMentions.candidates(
            in: roster, matching: query,
            excluding: coordinator.blockedUserIDs.union([coordinator.currentUserID]))
    }

    var body: some View {
        MacNoteEditor(
            text: $text, color: $color, size: $size, font: $font, event: $event,
            kind: kind,
            title: editTitle,
            mentionCandidates: mentionCandidates(matching:),
            lines: $lines,
            isDone: { itemID in
                if let sending = ticking[itemID] { return sending }
                return note.taskList.first { $0.id == itemID }?.done ?? false
            },
            onTick: { itemID, done in
                // One request per line at a time: a second click while the
                // first is in flight is the click that would undo it.
                guard ticking[itemID] == nil else { return }
                ticking[itemID] = done
                Task {
                    await coordinator.tickTask(
                        noteID: note.noteID, itemID: itemID, done: done)
                    ticking[itemID] = nil
                }
            }
        ) {
            let isEvent = kind == .event
            let isList = kind == .tasks
            let event = event
            let written = DraftTaskLine.written(lines)
            let held = note.taskList.map { APIClient.TaskLineRequest(id: $0.id, text: $0.text) }
            Task {
                // Size and font only when the author changed them, so a
                // name this Mac does not know survives a text edit
                // (NoteSize, NoteFont). An event's own three only on an
                // event — the server refuses them anywhere else — with the
                // end a DOUBLE option: none clears it.
                await coordinator.updateNote(
                    id: note.noteID, text: text, color: color,
                    size: size.patchName(replacing: note.size),
                    font: font.patchName(replacing: note.font),
                    startsAt: isEvent ? event.startsAt : nil,
                    endsAt: isEvent ? .some(event.hasEnd ? event.endsAt : nil) : nil,
                    place: isEvent ? event.trimmedPlace : nil,
                    // Sent only when they DIFFER: `items` is the author's
                    // field, and one sent unchanged would make opening a
                    // list to read it an edit.
                    items: isList && written != held ? written : nil)
            }
        }
        .onAppear {
            text = note.text
            color = note.color
            size = NoteSize(name: note.size)
            font = NoteFont(name: note.font)
            lines = note.taskList.map { DraftTaskLine(itemID: $0.id, text: $0.text) }
            if let starts = note.startsAt {
                event = MacEventFields(
                    startsAt: starts,
                    hasEnd: note.endsAt != nil,
                    endsAt: note.endsAt ?? starts.addingTimeInterval(3600),
                    place: note.place ?? "")
            }
        }
    }
}

#endif
