//
//  ReportInboxView.swift
//  FamilyConnect
//
//  The owner's moderation list.
//
//  The owner is the moderator, and that follows from the containment this
//  product already has: invite-code-only membership, approval by default,
//  no cross-family contact, no user directory, and an owner who can
//  already remove a member, reset their password, rotate the code and
//  close the family (docs/protocol.md, "Reporting a member").
//
//  What this screen deliberately does NOT show: who blocked whom. A family
//  owner is often a parent and the blocked person is often in the same
//  house, which is the exact case the silence exists for. Blocking and
//  reporting are independent — reporting does not block, blocking does not
//  report, and neither appears in the other's surface.
//
//  It also never shows a report ABOUT the owner: the server omits those
//  from `GET /families/reports` entirely, so there is nothing to filter
//  here. `Report.id` carries no ordering for the same reason — an owner
//  who saw ids 4, 6 and 7 would have been told that 5 exists.
//

import SwiftUI

struct ReportInboxView: View {
    @Environment(ChatSyncCoordinator.self) private var coordinator

    @State private var reports: [ReportDTO] = []
    @State private var isLoading = true
    @State private var resolving: Set<Int64> = []
    @State private var errorText: String?

    var body: some View {
        List {
            if reports.isEmpty && !isLoading {
                ContentUnavailableView(
                    "Nothing to review",
                    systemImage: "checkmark.shield",
                    description: Text("Reports from your family appear here."))
            }
            ForEach(reports) { report in
                Section {
                    row(report)
                }
            }
            if let errorText {
                Label(errorText, systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
        .navigationTitle("Reports")
        .task { await load() }
        .refreshable { await load() }
    }

    @ViewBuilder
    private func row(_ report: ReportDTO) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(ReportReason(rawValue: report.reason)?.label
                ?? ReportReason.other.label)
                .font(.headline)
            // Both names, because a moderator needs to know who is
            // complaining as well as about whom.
            Text("\(report.reporter.displayName) reported \(report.reported.displayName)")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            // The excerpt is drawn ALWAYS when present, even once the
            // message itself has been swept: it is frozen at the moment of
            // the report precisely because the author may edit the body
            // away and retention will delete it. An owner judging a
            // message has to see all of it, so it is never truncated here.
            if let excerpt = report.messageExcerpt, !excerpt.isEmpty {
                Text(excerpt)
                    .font(.callout)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
                    .textSelection(.enabled)
            }
        }
        .padding(.vertical, 2)

        Button {
            resolve(report)
        } label: {
            if resolving.contains(report.id) {
                ProgressView()
            } else {
                // Says nothing about what the owner DID: this protocol has
                // removing a member, resetting a password and closing the
                // family; it does not have deleting somebody else's
                // message. What "dealt with" means is the owner's business.
                Text("Mark as handled")
            }
        }
        .disabled(resolving.contains(report.id))
    }

    private func load() async {
        errorText = nil
        defer { isLoading = false }
        do {
            reports = try await coordinator.api.reports()
        } catch {
            errorText = String(localized: "Couldn't load the reports. Pull to refresh.")
        }
    }

    private func resolve(_ report: ReportDTO) {
        resolving.insert(report.id)
        Task {
            defer { resolving.remove(report.id) }
            do {
                try await coordinator.api.resolveReport(id: report.id)
                reports.removeAll { $0.id == report.id }
            } catch {
                errorText = String(localized: "Couldn't mark that as handled. Try again.")
            }
        }
    }
}
