//
//  StickerPackView.swift
//  FamilyConnect
//
//  The family's sticker pack, managed: every sticker in it, a way to add
//  one and a way to take one out (docs/protocol.md, "The pack").
//
//  It lives on the Family screen rather than in the panel, because the two
//  are different jobs: the panel is for SENDING, one tap and gone, and a
//  Remove button one tap away from Send is a sticker taken out of
//  everybody's panel by a slip of the thumb.
//
//  THE RULES, and where each is enforced:
//
//    * ANYBODY in the family may add. The picture goes up exactly as it is
//      when it is already a WebP or a PNG; anything else is fitted whole
//      into 512 × 512 and written as PNG (`StickerPack.make`). It never
//      goes through `MediaPrep`, whose photo path is a JPEG encoder.
//    * A picture that MOVES and is not a WebP — an animated GIF, an
//      animated PNG — is refused with a sentence, never made into a
//      sticker of its first frame.
//    * The few words that describe it are optional, at most 64 as the
//      server counts them, and an over-long one is refused in words on
//      the sheet, before anything is uploaded (`StickerPack.label`).
//    * Whoever ADDED a sticker, or the family OWNER, may remove it
//      (`StickerPack.canRemove`) — and nobody else is offered the action.
//    * The pack's two limits come from the server (`AppSettings`), and are
//      said here, beside the picker, rather than as a rejected request.
//
//  Shared by both platforms. The phone pushes it from Manage Family; the
//  Mac raises it as a sheet from the Family window.
//

import PhotosUI
import SwiftData
import SwiftUI
import UniformTypeIdentifiers

struct StickerPackView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss

    @Query(sort: \PackItemEntity.itemID) private var stored: [PackItemEntity]

    /// A picture that has been picked and turned into a sticker, waiting
    /// for its few words and the Add button.
    @State private var draft: StickerDraft?
    /// The sticker somebody asked to remove, while the question is up.
    @State private var removing: PackItemSnapshot?
    @State private var confirmRemove = false
    @State private var showsFiles = false
    #if os(iOS)
    @State private var showsPhotos = false
    @State private var pickedPhoto: PhotosPickerItem?
    #endif
    /// A one-line answer to something that could not be done — too big,
    /// full, not a picture, the server said no.
    @State private var notice: String?
    @State private var busy = false

    private let columns = [GridItem(.adaptive(minimum: 84), spacing: 10)]

    private var items: [PackItemSnapshot] { stored.map(\.snapshot) }
    private var limit: Int? { AppSettings.packMaxItems }
    private var hasRoom: Bool { StickerPack.hasRoom(count: stored.count, limit: limit) }

    var body: some View {
        content
            .navigationTitle("Family Stickers")
            .inlineNavigationTitle()
            .toolbar {
                // A Mac sheet's bar holds its closing action and little
                // else, so the way IN sits in the content there (above the
                // grid); a phone's navigation bar has the corner for it.
                #if os(macOS)
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
                #else
                ToolbarItem(placement: .primaryAction) { addControl }
                #endif
            }
            .fileImporter(
                isPresented: $showsFiles,
                // Any picture: a WebP or a PNG goes as it is, and any other
                // STILL one is made into a sticker. Greying out the JPEGs
                // would refuse the format a phone's library is full of. A
                // moving picture that is not a WebP can be picked too, and
                // is answered with a sentence (`MakeError.animatedNotWebP`)
                // — a greyed-out GIF says nothing about why.
                allowedContentTypes: [.image],
                allowsMultipleSelection: false
            ) { result in
                guard case .success(let urls) = result, let url = urls.first else { return }
                pick(fileAt: url)
            }
            #if os(iOS)
            .photosPicker(
                isPresented: $showsPhotos,
                selection: $pickedPhoto,
                matching: .images,
                // `.current`: the picker's default may hand over a JPEG
                // TRANSCODE of what is in the library, which is exactly the
                // re-encode a sticker must not get — an animated WebP would
                // arrive as its first frame on white.
                preferredItemEncoding: .current)
            .onChange(of: pickedPhoto) { _, item in
                guard let item else { return }
                pickedPhoto = nil
                pick(photo: item)
            }
            #endif
            .sheet(item: $draft) { draft in
                StickerAddSheet(
                    draft: draft,
                    onAdd: { label in add(draft, label: label) },
                    onCancel: { self.draft = nil })
            }
            .confirmationDialog(
                "Remove this sticker?",
                isPresented: $confirmRemove,
                titleVisibility: .visible,
                presenting: removing
            ) { item in
                Button("Remove", role: .destructive) { remove(item) }
            } message: { _ in
                Text("It leaves everyone's sticker panel. Stickers already sent stay in the chat.")
            }
        #if os(macOS)
            .frame(width: 520, height: 520)
        #endif
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                #if os(macOS)
                HStack {
                    Spacer()
                    addControl
                }
                #endif
                if let notice {
                    Label(notice, systemImage: "exclamationmark.circle")
                        .font(.callout)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityElement(children: .combine)
                }
                if stored.isEmpty {
                    ContentUnavailableView(
                        "No stickers yet",
                        systemImage: "face.smiling",
                        description: Text("Add a picture and everyone in the family can send it as a sticker."))
                        .padding(.top, 40)
                } else {
                    LazyVGrid(columns: columns, spacing: 10) {
                        ForEach(items) { item in cell(item) }
                    }
                }
                footer
            }
            .padding(16)
        }
        .overlay {
            if busy { ProgressView() }
        }
    }

    /// How full the pack is, and what adding does — said once, under the
    /// grid, where it is read after the stickers rather than instead.
    @ViewBuilder
    private var footer: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let limit {
                Text("\(stored.count) of \(limit) stickers")
            }
            Text("WebP and PNG pictures are added as they are. Anything else is fitted into 512 × 512.")
            Text("An animated sticker has to be a WebP picture.")
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func cell(_ item: PackItemSnapshot) -> some View {
        let removable = StickerPack.canRemove(
            addedBy: item.addedBy,
            currentUserID: coordinator.currentUserID,
            isOwner: session.isOwner)
        return StickerImage(attachmentID: item.attachmentID, animates: false)
            .frame(maxWidth: .infinity)
            .aspectRatio(1, contentMode: .fit)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(StickerPanel.accessibilityName(item))
            .accessibilityAddTraits(.isImage)
            .padding(6)
            .background(
                Color.appSecondaryFill.opacity(0.5),
                in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(alignment: .topTrailing) {
                // A visible control as well as the context menu: a long
                // press is undiscoverable, and on a Mac a right click is
                // not something a screen reader's user is told exists.
                if removable {
                    Button {
                        askToRemove(item)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 18))
                            .symbolRenderingMode(.palette)
                            .foregroundStyle(.white, .secondary)
                            .padding(4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(busy)
                    .accessibilityLabel("Remove sticker")
                }
            }
            .contextMenu {
                if removable {
                    Button("Remove", role: .destructive) { askToRemove(item) }
                }
            }
    }

    /// The door in. One button where there is one source (the Mac's open
    /// panel reaches the Photos library through its own sidebar), a menu of
    /// two where there are two.
    @ViewBuilder
    private var addControl: some View {
        #if os(iOS)
        Menu {
            Button {
                guard checkRoom() else { return }
                showsPhotos = true
            } label: {
                Label("Photo Library", systemImage: "photo.on.rectangle")
            }
            Button {
                guard checkRoom() else { return }
                showsFiles = true
            } label: {
                Label("File", systemImage: "doc")
            }
        } label: {
            Image(systemName: "plus")
        }
        .disabled(busy)
        .accessibilityLabel("Add a sticker")
        #else
        Button {
            guard checkRoom() else { return }
            showsFiles = true
        } label: {
            Label("Add a sticker", systemImage: "plus")
        }
        .disabled(busy)
        #endif
    }

    // MARK: - Adding

    /// A full pack is said BEFORE the picker opens: choosing a picture and
    /// then being told there was never room for it is the refusal arriving
    /// a screen too late.
    private func checkRoom() -> Bool {
        guard hasRoom else {
            notice = ChatSyncCoordinator.packFailure(
                APIError.conflict(code: "pack_full", message: nil))
            return false
        }
        notice = nil
        return true
    }

    private func pick(fileAt url: URL) {
        stage { StickerPack.read(fileAt: url) }
    }

    #if os(iOS)
    private func pick(photo item: PhotosPickerItem) {
        stage { try? await item.loadTransferable(type: Data.self) }
    }
    #endif

    /// Read the picked bytes, turn them into a sticker and put it up for
    /// its label. `StickerPack.make` and NOT `MediaPrep`: see the file
    /// header.
    ///
    /// OFF THE MAIN ACTOR, both halves, the way the photo path prepares a
    /// photograph. Somebody picking a sticker picks a photograph as often
    /// as not, and making one out of a 48-megapixel HEIC is a full decode
    /// and then a PNG written once per rung of the ladder until one fits —
    /// up to six. Done where the taps are handled, that is a screen that
    /// does not answer for as long as it takes; done elsewhere it is a
    /// spinner. The file is read there too: a picture on a network volume
    /// or in iCloud Drive is not read in a frame.
    private func stage(_ read: @escaping @Sendable () async -> Data?) {
        let ceiling = AppSettings.packMaxItemBytes ?? Int.max
        busy = true
        Task {
            let result = await StickerPack.prepare(maxBytes: ceiling, read: read)
            busy = false
            switch result {
            case .success(let prepared):
                draft = StickerDraft(made: prepared.made, still: prepared.still)
                notice = nil
            case .failure(.tooLarge):
                notice = ChatSyncCoordinator.packFailure(APIError.payloadTooLarge)
            case .failure(.animatedNotWebP):
                // Said, not flattened: the only sticker this device could
                // make of a moving GIF is its first frame.
                notice = StickerPack.animatedNotWebPNotice
            case .failure(.unreadable):
                notice = String(localized: "Couldn't read that picture.")
            }
        }
    }

    private func add(_ draft: StickerDraft, label: String) {
        self.draft = nil
        busy = true
        Task {
            let outcome = await coordinator.addSticker(draft.made, label: label)
            busy = false
            switch outcome {
            case .added:
                notice = nil
            case .alreadyHeld:
                notice = String(localized: "The family already has that sticker.")
            case .failed(let message):
                notice = message
            }
        }
    }

    // MARK: - Removing

    private func askToRemove(_ item: PackItemSnapshot) {
        removing = item
        confirmRemove = true
    }

    private func remove(_ item: PackItemSnapshot) {
        busy = true
        Task {
            notice = await coordinator.removeSticker(id: item.id)
            busy = false
        }
    }
}

/// A sticker that has been made and not yet added.
struct StickerDraft: Identifiable {
    let id = UUID()
    let made: StickerPack.Made
    /// Frame zero, decoded ONCE where the sticker was made, for the label
    /// sheet to show until (and unless) the player has a frame. Decoding it
    /// in the sheet's body meant decoding it again on every keystroke in
    /// the field beside it.
    let still: CGImage?
}

/// The last step of adding: see what will be added, and say in a few words
/// what it is — for somebody who cannot see it (docs/protocol.md: "a few
/// words for a screen reader, never drawn over the picture"). Optional, and
/// fixed once added; there is no edit.
struct StickerAddSheet: View {
    let draft: StickerDraft
    let onAdd: (String) -> Void
    let onCancel: () -> Void

    @State private var label = ""
    @State private var player = StickerPlayer()

    /// Over the server's 64, counted as the server counts — trimmed, in
    /// Unicode scalars (`StickerPack.label`).
    private var labelTooLong: Bool { StickerPack.label(label) == .tooLong }

    private var still: Image? {
        draft.still.map { PlatformImage.view($0) }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                Group {
                    if let frame = player.frame {
                        Image(decorative: frame, scale: 1).resizable()
                    } else if let still {
                        still.resizable()
                    }
                }
                .aspectRatio(contentMode: .fit)
                .frame(width: 160, height: 160)
                .accessibilityHidden(true)

                TextField("Description (optional)", text: $label)
                    .textFieldStyle(.roundedBorder)
                // REFUSED IN WORDS, here, before any request — not cut. The
                // field used to clamp itself on every keystroke, which
                // turned a pasted description into a shorter one without a
                // word; and a label is fixed once added. So the field holds
                // what was typed, this says why it cannot go, and Add waits.
                if labelTooLong {
                    Text(StickerPack.labelTooLongNotice)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text("A few words for people who use a screen reader.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Spacer(minLength: 0)
            }
            .padding(20)
            .navigationTitle("Add a sticker")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { onAdd(label) }
                        .disabled(labelTooLong)
                }
            }
        }
        #if os(macOS)
        .frame(width: 360, height: 340)
        #else
        .presentationDetents([.medium])
        #endif
        .task { player.play(draft.made.data) }
        .onDisappear { player.stop() }
    }
}
