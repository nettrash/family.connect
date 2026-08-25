//
//  BirthdayView.swift
//  FamilyConnect
//
//  Two sheets over the same idea: setting a day and a month.
//
//  MyBirthdayView is your own, from Settings, and needs no permission
//  from anybody. MemberBirthdayView is the owner filling one in for
//  somebody else from the roster — which is what makes a family calendar
//  usable at all, because a parent knows a child's birthday and the child
//  is never going to open a settings screen to type it (protocol.md,
//  "Birthdays").
//
//  There is no year, in either. The pickers cannot offer one, so nobody
//  can publish their age by accident, and 29 February is a perfectly good
//  birthday because there is no year for it to fail to exist in.
//
//  The day picker is bounded by the month, so an impossible date cannot be
//  chosen in the first place — but the SERVER is what decides which dates
//  exist, so a `validation` refusal is still shown rather than swallowed.
//
//  Android counterpart: ui/settings/BirthdayScreen.kt
//

import SwiftUI

/// Shared shape: a month, a day the month has, and the rule that keeps
/// the second inside the first.
private struct BirthdayFields: View {
    @Binding var month: Int
    @Binding var day: Int

    private let monthNames = BirthdayDTO.monthNames()

    var body: some View {
        // Both non-localizing on purpose: month names come from the OS
        // calendar in the reader's language, and a day is a number.
        Picker("Month", selection: $month) {
            ForEach(Array(monthNames.enumerated()), id: \.offset) { index, name in
                Text(name).tag(index + 1)
            }
        }
        Picker("Day", selection: $day) {
            // Through `dayRange`, which cannot invert: `1...daysIn(month:)`
            // trapped on a month the calendar does not have, and the sheet
            // is opened straight from a roster row.
            ForEach(BirthdayDTO.dayRange(forMonth: month), id: \.self) { value in
                Text(value.formatted()).tag(value)
            }
        }
        .onChange(of: month) { _, newMonth in
            // 31 March → April has to land somewhere, and the last day of
            // the new month is the only answer that isn't a surprise.
            day = min(day, BirthdayDTO.daysIn(month: newMonth))
        }
    }
}

/// What the two sheets say when a save comes back refused. The server
/// owns the day-vs-month rule, so its "no" is reported rather than
/// second-guessed.
private enum BirthdayFailure {
    static func message(for error: Error) -> String {
        if case APIError.conflict(let code, _) = error, code == "validation" {
            return String(localized: "That date doesn't exist.")
        }
        if case APIError.forbidden = error {
            return String(localized: "Only the family owner can do that.")
        }
        return String(localized: "Couldn't save that birthday. Try again.")
    }
}

/// Your own birthday.
struct MyBirthdayView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var month = 1
    @State private var day = 1
    @State private var isSaving = false
    @State private var errorText: String?

    private var existing: BirthdayDTO? { session.currentUser?.birthday }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    BirthdayFields(month: $month, day: $day)
                } footer: {
                    Text("A day and a month, with no year — so being wished a happy birthday never means publishing your age.")
                }
                if existing != nil {
                    Section {
                        Button("Remove Birthday", role: .destructive) { clear() }
                            .disabled(isSaving)
                    }
                }
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "exclamationmark.circle")
                            .font(.callout)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Birthday")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }
                        .disabled(isSaving)
                }
            }
            .overlay {
                if isSaving { ProgressView() }
            }
        }
        .interactiveDismissDisabled(isSaving)
        .onAppear {
            // Clamped on the way in: what a roster row says is not
            // necessarily a date, and the pickers below can only show one
            // that exists.
            if let existing = existing?.clamped {
                month = existing.month
                day = existing.day
            }
        }
    }

    private func save() {
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                let user = try await coordinator.api.setMyBirthday(month: month, day: day)
                apply(user)
                dismiss()
            } catch {
                errorText = BirthdayFailure.message(for: error)
            }
        }
    }

    private func clear() {
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                try await coordinator.api.clearMyBirthday()
                // DELETE answers 204, so there is no user to apply —
                // build the cleared one, exactly as removing an avatar does.
                if let user = session.currentUser {
                    apply(UserDTO(
                        id: user.id,
                        username: user.username,
                        displayName: user.displayName,
                        createdAt: user.createdAt,
                        avatarVersion: user.avatarVersion,
                        birthday: nil))
                }
                dismiss()
            } catch {
                errorText = BirthdayFailure.message(for: error)
            }
        }
    }

    /// Both copies of "me": the session's profile, which Settings draws,
    /// and my own roster row, which the Mac's family sheet draws.
    private func apply(_ user: UserDTO) {
        session.applyProfile(user)
        coordinator.applyMemberBirthday(userID: user.id, birthday: user.birthday)
    }
}

/// The owner setting somebody else's — including, deliberately, their own
/// row: the protocol allows the roster endpoint to name the owner so that
/// no roster screen has to carry a special case for exactly one person.
struct MemberBirthdayView: View {
    let member: MemberDTO
    /// The new value, or nil once it is cleared. The caller patches the
    /// row it holds — there is no frame and no push for this, so nothing
    /// else will tell it.
    var onSaved: (BirthdayDTO?) -> Void

    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss

    @State private var month = 1
    @State private var day = 1
    @State private var isSaving = false
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    BirthdayFields(month: $month, day: $day)
                } header: {
                    Text("Birthday for \(member.displayName)")
                } footer: {
                    Text("A day and a month, with no year. Everyone in the family sees it.")
                }
                if member.birthday != nil {
                    Section {
                        Button("Remove Birthday", role: .destructive) { clear() }
                            .disabled(isSaving)
                    }
                }
                if let errorText {
                    Section {
                        Label(errorText, systemImage: "exclamationmark.circle")
                            .font(.callout)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Birthday")
            .inlineNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save() }
                        .disabled(isSaving)
                }
            }
            .overlay {
                if isSaving { ProgressView() }
            }
        }
        .interactiveDismissDisabled(isSaving)
        .onAppear {
            // Clamped on the way in, for the reason MyBirthdayView says.
            if let existing = member.birthday?.clamped {
                month = existing.month
                day = existing.day
            }
        }
    }

    private func save() {
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                let updated = try await coordinator.api.setMemberBirthday(
                    userID: member.id, month: month, day: day)
                apply(updated.birthday)
                dismiss()
            } catch {
                errorText = BirthdayFailure.message(for: error)
            }
        }
    }

    private func clear() {
        errorText = nil
        isSaving = true
        Task {
            defer { isSaving = false }
            do {
                try await coordinator.api.clearMemberBirthday(userID: member.id)
                apply(nil)
                dismiss()
            } catch {
                errorText = BirthdayFailure.message(for: error)
            }
        }
    }

    /// The local roster row too, not only the caller's list: the Mac's
    /// family sheet reads SwiftData while the phone's reads the DTOs
    /// straight off the API, and the two must not disagree.
    private func apply(_ birthday: BirthdayDTO?) {
        coordinator.applyMemberBirthday(userID: member.id, birthday: birthday)
        onSaved(birthday)
    }
}
