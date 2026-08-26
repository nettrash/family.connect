//
//  PollComposerView.swift
//  FamilyConnect
//
//  The small form behind the composer's "Poll" item: a question and
//  between two and ten options.
//
//  Shared, with no `#if os` guard around the form itself — both platforms
//  ask for exactly the same three things — and a FIXED SIZE on the Mac,
//  because a macOS sheet cannot be resized by the person using it (the
//  lesson MacSettingsView's frame already records). The chrome differs and
//  only the chrome: a navigation bar with Cancel/Create on the phone, a
//  title row and a button bar on the Mac.
//
//  The rules it enforces are the protocol's, so the form refuses locally
//  exactly what the server would refuse with `invalid_poll`: 2–10 options,
//  each trimmed, non-empty and at most 100 characters, no two the same
//  ignoring case, and a question that is not empty — a poll's body, unlike
//  a message carrying an attachment, may not be blank (docs/protocol.md,
//  "Polls").
//

import SwiftUI

struct PollComposerView: View {
    /// Called with the question and the cleaned options when Create is
    /// pressed. The caller sends; this view only collects.
    let onCreate: (String, [String]) -> Void
    let onCancel: () -> Void

    @State private var question = ""
    /// Starts at the minimum, because a poll with one option is not a
    /// poll and an empty list looks broken.
    @State private var options: [Option] = [Option(), Option()]
    @FocusState private var focusedField: Field?

    /// A stable identity per row, so adding or removing one does not make
    /// SwiftUI re-key the fields and move the focus somewhere else.
    private struct Option: Identifiable {
        let id = UUID()
        var text = ""
    }

    private enum Field: Hashable {
        case question
        case option(UUID)
    }

    /// The cleaned options, or nil while the form is not yet a legal poll.
    private var sanitized: [String]? {
        PollPresentation.sanitizedOptions(options.map(\.text))
    }

    private var canCreate: Bool {
        !question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && sanitized != nil
    }

    var body: some View {
        #if os(macOS)
        VStack(alignment: .leading, spacing: 0) {
            Text("New poll")
                .font(.headline)
                .padding(.horizontal, 20)
                .padding(.top, 20)
            form
            Divider()
            HStack {
                Spacer()
                Button("Cancel") { onCancel() }
                    .keyboardShortcut(.cancelAction)
                Button("Create") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canCreate)
            }
            .padding(16)
        }
        // A Mac sheet cannot be resized by the person using it, so it is
        // sized here for the tallest thing it can hold: ten options plus
        // the question. Grow BOTH numbers if a row is ever added, or the
        // last one hides behind a scroll.
        .frame(width: 460, height: 560)
        #else
        NavigationStack {
            form
                .navigationTitle("New poll")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { onCancel() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Create") { create() }
                            .disabled(!canCreate)
                    }
                }
        }
        #endif
    }

    private var form: some View {
        Form {
            Section {
                TextField("Ask the family something…", text: $question, axis: .vertical)
                    .lineLimit(1...3)
                    .focused($focusedField, equals: .question)
            } header: {
                Text("Question")
            } footer: {
                // Says the one thing that is not obvious from the form:
                // the question becomes the message, so everybody sees it
                // even on a client that knows nothing of polls.
                Text("The question is the message everyone sees.")
            }

            Section {
                ForEach($options) { $option in
                    HStack {
                        TextField("Option", text: $option.text)
                            .focused($focusedField, equals: .option(option.id))
                        if options.count > PollPresentation.minOptions {
                            Button {
                                remove(option.id)
                            } label: {
                                Image(systemName: "minus.circle.fill")
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Remove option")
                        }
                    }
                }
                if options.count < PollPresentation.maxOptions {
                    Button {
                        addOption()
                    } label: {
                        Label("Add option", systemImage: "plus.circle.fill")
                    }
                    .buttonStyle(.plain)
                }
            } header: {
                Text("Options")
            } footer: {
                Text("Between 2 and 10 options. They can't be changed once the poll is sent.")
            }
        }
        #if os(macOS)
        .formStyle(.grouped)
        #endif
    }

    private func addOption() {
        let option = Option()
        options.append(option)
        focusedField = .option(option.id)
    }

    private func remove(_ id: UUID) {
        guard options.count > PollPresentation.minOptions else { return }
        options.removeAll { $0.id == id }
    }

    private func create() {
        // Sanitised once, and the SAME list is what leaves — validating
        // one thing and sending another is how a form ships an option the
        // server refuses.
        guard let sanitized else { return }
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        onCreate(trimmed, sanitized)
    }
}
