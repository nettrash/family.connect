//
//  StickerViewer.swift
//  FamilyConnect
//
//  A sticker from a chat, shown larger — and, when the family's pack does
//  not hold it, the offer to keep it: "Add to family stickers"
//  (docs/protocol.md, "Tapping one shows it larger").
//
//  Not the photo viewer. That one pages, zooms and shares, which is what a
//  photograph is for; a sticker is a small picture with one thing to do
//  about it, and a full-screen black pager around a 512-pixel cut-out is
//  the wrong room.
//
//  WHETHER THE PACK HOLDS IT is decided HERE, from bytes this device
//  already has — nothing on the wire names the pack item a message was
//  sent from, and a message outlives the item: somebody may have removed
//  it from the pack since. A wrong guess is harmless either way: the
//  server answers an add of bytes it already holds with `200` and the item
//  that was there (`StickerPack.holds`).
//
//  Shared by both platforms, as a sheet on each.
//

import SwiftUI

struct StickerViewer: View {
    /// The message's own attachment — its bytes are the sticker.
    let attachment: AttachmentDTO

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(AttachmentStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    private enum Offer: Equatable {
        /// Still finding out — the bytes have not arrived.
        case unknown
        /// The pack holds it; there is nothing to offer.
        case held
        /// The pack does not. The button is up.
        case offered
        case adding
        /// Just added, by this tap or by somebody else meanwhile.
        case added
        case failed(String)
    }

    @State private var offer: Offer = .unknown

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                StickerImage(attachmentID: attachment.id)
                    .frame(maxWidth: 320, maxHeight: 320)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Sticker")
                    .accessibilityAddTraits(.isImage)
                footer
                Spacer(minLength: 0)
            }
            .padding(24)
            .frame(maxWidth: .infinity)
            .navigationTitle("Sticker")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
        }
        #if os(macOS)
        .frame(width: 420, height: 460)
        #else
        .presentationDetents([.medium, .large])
        #endif
        // Re-asked when a fetch lands: the answer needs the bytes, and the
        // first pass may be the one that started fetching them.
        .task(id: store.generation) { await decide() }
    }

    @ViewBuilder
    private var footer: some View {
        switch offer {
        case .unknown, .held:
            EmptyView()
        case .offered:
            Button {
                add()
            } label: {
                Label("Add to family stickers", systemImage: "plus.circle")
            }
            .buttonStyle(.borderedProminent)
        case .adding:
            ProgressView()
        case .added:
            Label("Added to family stickers", systemImage: "checkmark.circle")
                .foregroundStyle(.secondary)
        case .failed(let message):
            VStack(spacing: 10) {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Add to family stickers") { add() }
            }
        }
    }

    /// Hold it, or offer it. Only ever moves OUT of `.unknown`: a verdict
    /// already reached — above all "added" — is not second-guessed by a
    /// redraw.
    ///
    /// The comparison is bytes against bytes, for every pack item of the
    /// same type and size, and it is done OFF the main actor
    /// (`StickerPack.holds(fileAt:)`): it is the sheet's first frame that
    /// would otherwise wait for it.
    private func decide() async {
        guard offer == .unknown else { return }
        // A server with no packs has nowhere to add to.
        guard AppSettings.offersStickers else { return }
        let files = Dictionary(
            coordinator.packItems().map { ($0, store.originalURL(id: $0.attachmentID)) },
            uniquingKeysWith: { first, _ in first })
        let held = await StickerPack.holds(
            fileAt: store.originalURL(id: attachment.id),
            mime: attachment.mime,
            // A pending row's provisional attachment carries no size.
            size: attachment.size > 0 ? attachment.size : nil,
            items: Array(files.keys),
            fileOf: { files[$0] ?? URL(fileURLWithPath: "/dev/null") })
        // nil: the bytes have not arrived. The next `generation` asks again.
        guard let held, offer == .unknown else { return }
        offer = held ? .held : .offered
    }

    /// The pack's own flow: the message's bytes, uploaded again exactly as
    /// they are, and claimed. No `MediaPrep`, and no re-encode even through
    /// `StickerPack.make` — these bytes already ARE a sticker.
    private func add() {
        offer = .adding
        Task {
            guard let bytes = await store.cachedBytes(id: attachment.id) else {
                offer = .offered
                return
            }
            let made = StickerPack.Made(
                data: bytes, mime: attachment.mime,
                width: attachment.width, height: attachment.height)
            switch await coordinator.addSticker(made, label: nil) {
            case .added, .alreadyHeld:
                // `alreadyHeld` is the wrong guess the protocol allows for,
                // answered `200`: the family has it, which is what was
                // asked for.
                offer = .added
            case .failed(let message):
                offer = .failed(message)
            }
        }
    }
}
