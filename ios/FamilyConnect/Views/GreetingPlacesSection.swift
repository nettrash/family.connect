//
//  GreetingPlacesSection.swift
//  FamilyConnect
//
//  The owner's places for the daily greeting's weather (protocol.md,
//  "Today's weather, for places the owner chose"): up to three place names,
//  each its own field, under the "Daily greeting" switch in
//  FamilyAssistantSettings — so it is on the phone's Manage Family screen
//  and the Mac's family sheet alike.
//
//  The caller decides whether it is drawn at all (`GreetingPlaces.isOffered`);
//  this decides only what the list looks like and when it is saved.
//
//  SAVED WHOLE, when the owner is done with a field — Return, or focus
//  leaving every field, or a place removed, or the screen going away — and
//  not on every keystroke: the server REPLACES the list, and a PATCH per
//  letter would put half-typed names on the wire to no purpose. A list the
//  server already holds is not sent again.
//
//  The fields then show the list AS THE SERVER KEPT IT — trimmed, repeats
//  dropped, an unfilled field gone — because that, not what was typed, is
//  what the greeting will use. Never while a field is being typed in:
//  replacing a field under the owner's cursor would lose their place.
//

import SwiftUI

struct GreetingPlacesSection: View {
    /// The list as the server last answered it.
    let stored: [String]
    /// A write — this one or another switch's — is in flight.
    let isSaving: Bool
    /// Send this list (already as the server will keep it).
    var onSave: ([String]) -> Void
    /// The list could not be sent as it stands. Unreachable through the
    /// fields, which hold no control character and stop at the limit, but a
    /// refusal must never be silent.
    var onInvalid: () -> Void

    private struct Draft: Identifiable, Equatable {
        let id = UUID()
        var text: String
    }

    @State private var drafts: [Draft]
    /// The list most recently handed to `onSave`, while that write is in
    /// flight — so Return followed by the focus leaving does not send the
    /// same list twice.
    @State private var sending: [String]?
    @FocusState private var focused: UUID?

    init(
        stored: [String], isSaving: Bool,
        onSave: @escaping ([String]) -> Void, onInvalid: @escaping () -> Void
    ) {
        self.stored = stored
        self.isSaving = isSaving
        self.onSave = onSave
        self.onInvalid = onInvalid
        _drafts = State(initialValue: stored.map { Draft(text: $0) })
    }

    var body: some View {
        Section {
            ForEach($drafts) { $draft in
                HStack {
                    TextField("City or town", text: limited($draft))
                        .focused($focused, equals: draft.id)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textContentType(.addressCity)
                        .submitLabel(.done)
                        #endif
                        .onSubmit { commit() }
                    Button {
                        remove(draft.id)
                    } label: {
                        Image(systemName: "minus.circle.fill")
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("Remove place")
                    .disabled(isSaving)
                }
            }
            if GreetingPlaces.canAdd(count: drafts.count) {
                Button {
                    add()
                } label: {
                    Label("Add place", systemImage: "plus.circle.fill")
                }
                .disabled(isSaving)
            } else {
                Text("Up to 3 places.")
                    .foregroundStyle(.secondary)
            }
            caption
        } header: {
            Text("Weather in the greeting")
        } footer: {
            footer
        }
        .onChange(of: stored) { _, kept in
            sending = nil
            if focused == nil { reseed(kept) }
        }
        .onChange(of: isSaving) { _, saving in
            // A write that failed leaves `stored` alone; without this the
            // same list could never be retried.
            if !saving { sending = nil }
        }
        .onChange(of: focused) { _, now in
            if now == nil { commit() }
        }
        .onDisappear { commit() }
    }

    // MARK: - The explanation

    private var explanation: String {
        String(localized: "The daily greeting will also mention today's weather in these places. Only the place names are sent to Open-Meteo to fetch the forecast. Nothing else is sent.")
    }

    /// The Mac's spelling — a caption inside the section, as the switches
    /// above it explain themselves there.
    @ViewBuilder
    private var caption: some View {
        #if os(macOS)
        Text(explanation)
            .font(.caption)
            .foregroundStyle(.secondary)
        #endif
    }

    /// The phone's — a section footer.
    @ViewBuilder
    private var footer: some View {
        #if os(iOS)
        Text(explanation)
        #endif
    }

    // MARK: - Editing

    private func add() {
        guard GreetingPlaces.canAdd(count: drafts.count) else { return }
        let draft = Draft(text: "")
        drafts.append(draft)
        focused = draft.id
    }

    /// The field's text, cut to the server's bound as it is typed. Its own
    /// function, with its types written out, so the `ForEach` row above is
    /// not one long expression for the type checker to solve — CI's Xcode
    /// gave up on a smaller one before (PackSyncTests).
    private func limited(_ draft: Binding<Draft>) -> Binding<String> {
        Binding<String>(
            get: { draft.wrappedValue.text },
            set: { (typed: String) in draft.wrappedValue.text = GreetingPlaces.limitInput(typed) })
    }

    private func remove(_ id: UUID) {
        if focused == id { focused = nil }
        drafts.removeAll { $0.id == id }
        commit()
    }

    private func reseed(_ kept: [String]) {
        let texts = drafts.map(\.text)
        guard !GreetingPlaces.same(texts, kept) else { return }
        drafts = kept.map { Draft(text: $0) }
    }

    /// Send the list if it differs from what the server holds; otherwise
    /// tidy the fields back to the server's spelling.
    private func commit() {
        switch GreetingPlaces.prepare(drafts.map(\.text)) {
        case .success(let kept):
            // Compared scalar by scalar, as the server compares.
            if GreetingPlaces.same(kept, stored) {
                if focused == nil { reseed(kept) }
            } else if !GreetingPlaces.same(kept, sending) {
                sending = kept
                onSave(kept)
            }
        case .failure:
            onInvalid()
        }
    }
}
