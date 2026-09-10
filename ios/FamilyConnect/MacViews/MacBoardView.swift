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
    /// Pinning a picture: the upload it turns into, and what went wrong.
    @State private var pinning = false
    @State private var pinFailure: String?

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .topLeading) {
                Color(nsColor: .underPageBackgroundColor)
                ForEach(notes) { note in
                    sticker(for: note, board: geometry.size)
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
                title: composingKind == .event ? "New Event" : "New Note"
            ) {
                let isEvent = composingKind == .event
                let event = draftEvent
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
                        place: isEvent && !event.trimmedPlace.isEmpty ? event.trimmedPlace : nil)
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
            })
    }

    /// A blank note — or event — to write. An event starts on the next
    /// round hour and is blue, as on the phone; a note is any colour.
    private func startComposing(_ kind: NoteKind) {
        composingKind = kind
        draftText = ""
        draftColor = kind == .event ? "blue" : (NoteColor.palette.randomElement() ?? "yellow")
        draftSize = .medium
        draftFont = .plain
        draftEvent = MacEventFields()
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
            (isHidden ? Text("Hidden — blocked member") : Text(note.text))
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
    let onSave: () -> Void

    private var canSave: Bool {
        kind == .photo || !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
            if NoteText.shouldShowCounter(text) {
                Text("\(NoteText.remaining(text)) characters left")
                    .font(.caption)
                    .foregroundStyle(NoteText.remaining(text) == 0 ? .red : .secondary)
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
                    text: text.isEmpty ? String(localized: "Your note") : text,
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
    @State private var text: String = ""
    @State private var color: String = "yellow"
    @State private var size: NoteSize = .medium
    @State private var font: NoteFont = .plain
    @State private var event = MacEventFields()

    private var kind: NoteKind { NoteKind(name: note.kind) }

    var body: some View {
        MacNoteEditor(
            text: $text, color: $color, size: $size, font: $font, event: $event,
            kind: kind,
            title: kind == .event ? "Edit Event" : "Edit Note"
        ) {
            let isEvent = kind == .event
            let event = event
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
                    place: isEvent ? event.trimmedPlace : nil)
            }
        }
        .onAppear {
            text = note.text
            color = note.color
            size = NoteSize(name: note.size)
            font = NoteFont(name: note.font)
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
