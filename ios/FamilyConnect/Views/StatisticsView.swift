//
//  StatisticsView.swift
//  FamilyConnect
//
//  What the family has actually sent (docs/protocol.md, "Family
//  statistics"). Every member sees the same numbers — it is a shared
//  curiosity, not an owner's dashboard.
//
//  Platform-free: the same screen on iOS and macOS, presented as a sheet by
//  both. Nothing here is cached — it is a page you open occasionally, and a
//  stale count would be worse than a moment's spinner.
//

import SwiftUI

struct StatisticsView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var stats: FamilyStatsDTO?
    @State private var failed = false

    var body: some View {
        Group {
            if let stats {
                content(stats)
            } else if failed {
                ContentUnavailableView(
                    "Couldn't load statistics",
                    systemImage: "chart.bar",
                    description: Text("Check your connection and try again."))
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Statistics")
        .inlineNavigationTitle()
        .task { await load() }
        #if os(macOS)
        .frame(width: 520, height: 560)
        .safeAreaInset(edge: .bottom) {
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(12)
        }
        #endif
    }

    @ViewBuilder
    private func content(_ stats: FamilyStatsDTO) -> some View {
        Form {
            Section("The family") {
                LabeledContent("Members", value: "\(stats.totals.members)")
                LabeledContent("Messages", value: "\(stats.totals.messages)")
                LabeledContent("Board notes", value: "\(stats.totals.boardNotes)")
            }

            Section("Attachments") {
                LabeledContent("Photos", value: "\(stats.totals.attachments.photo)")
                LabeledContent("Videos", value: "\(stats.totals.attachments.video)")
                LabeledContent("Audio", value: "\(stats.totals.attachments.audio)")
                LabeledContent("Files", value: "\(stats.totals.attachments.file)")
                LabeledContent("Sent", value: Self.bytes(stats.totals.attachments.bytes))
                if let stored = stats.totals.attachments.storedBytes {
                    LabeledContent("On disk", value: Self.bytes(stored))
                    let saved = stats.totals.attachments.bytes - stored
                    if saved > 0 {
                        // The gap is the point: identical bytes are kept
                        // once per family, so this is what that saved.
                        Text("\(Self.bytes(saved)) saved by storing one copy of identical files.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if stats.totals.ai.questions > 0 || stats.totals.ai.images > 0 {
                Section("Assistant") {
                    LabeledContent("Questions", value: "\(stats.totals.ai.questions)")
                    LabeledContent(
                        "Tokens",
                        value: "\(stats.totals.ai.promptTokens + stats.totals.ai.completionTokens)")
                    // Its own row, next to the tokens rather than folded
                    // into them, because it is a different bill: an image
                    // model reports no tokens at all, so a family shown
                    // only the two numbers above would see the expensive
                    // half of their assistant as free (protocol.md,
                    // "Family statistics"). Hidden at zero, which is what
                    // every server that has never drawn one reports —
                    // including every server that cannot.
                    if stats.totals.ai.images > 0 {
                        LabeledContent("Pictures", value: "\(stats.totals.ai.images)")
                    }
                }
            }

            Section("Who sends what") {
                ForEach(stats.members) { member in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack {
                            Text(member.displayName)
                                .font(.body.weight(.medium))
                            Spacer()
                            Text("\(member.messages)")
                                .font(.body.monospacedDigit())
                        }
                        Text(Self.summary(for: member))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 2)
                }
            }
        }
        .formStyle(.grouped)
    }

    /// One line under a member: what they sent besides words.
    private static func summary(for member: MemberStatsDTO) -> String {
        var parts: [String] = []
        if member.attachments.count > 0 {
            parts.append(String(
                localized: "\(member.attachments.count) attachments, \(bytes(member.attachments.bytes))"))
        }
        if member.ai.questions > 0 {
            parts.append(String(localized: "\(member.ai.questions) questions to the assistant"))
        }
        if member.ai.images > 0 {
            parts.append(String(localized: "\(member.ai.images) pictures from the assistant"))
        }
        if parts.isEmpty { return String(localized: "Words only") }
        return parts.joined(separator: " · ")
    }

    /// `1.2 MB`, in the reader's own units and language.
    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    private func load() async {
        failed = false
        do {
            stats = try await coordinator.api.familyStats()
        } catch {
            failed = true
        }
    }
}
