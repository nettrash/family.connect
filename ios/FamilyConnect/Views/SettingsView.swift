//
//  SettingsView.swift
//  FamilyConnect
//
//  Profile, server, family membership and session controls. Family
//  details (including the owner-only invite_code) come fresh from
//  GET /families/mine in .task rather than from cached session state,
//  because the invite code can be rotated from another device and stale
//  codes actively mislead.
//
//  Both destructive actions confirm first. Leave is special-cased for
//  the owner: the server answers 409 owner_cannot_leave unless they are
//  the sole member, and the alert explains that instead of parroting an
//  error code.
//

// iOS only — the Mac has its own views (MacViews/).
#if os(iOS)

import Observation
import OSLog
import PhotosUI
import SwiftUI

@MainActor @Observable
final class SettingsModel {
    var family: FamilyDTO?
    var members: [MemberDTO] = []
    var isLoading = false
    var confirmLeave = false
    var confirmLogout = false
    var showsStatistics = false
    /// Who would inherit if the owner left right now — a PREDICTION, and
    /// only ever read straight after a fresh `GET /families/mine`. Any
    /// join or leave changes the answer and none of them raises a frame
    /// for it (docs/protocol.md, `GET /families/mine`).
    var nextOwnerUserID: Int64?
    /// Who the family actually went to, once it has gone. Shown before
    /// this screen closes.
    var handedOverTo: String?
    var errorText: String?

    func load(api: APIClient) async {
        isLoading = true
        defer { isLoading = false }
        do {
            let mine = try await api.myFamily()
            family = mine.family
            members = mine.members
            nextOwnerUserID = mine.nextOwnerUserID
        } catch {
            // Non-fatal: the cached session family still renders.
        }
    }
}

struct SettingsView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator
    @Environment(\.dismiss) private var dismiss
    @State private var model = SettingsModel()
    /// Mirrors AppSettings.linkPreviewsEnabled — defaults are not
    /// observable, so the toggle owns the state and writes through.
    @State private var linkPreviewsEnabled = AppSettings.linkPreviewsEnabled
    @State private var mapPreviewsEnabled = AppSettings.mapPreviewsEnabled
    @State private var pickedPhoto: PhotosPickerItem?
    @State private var uploadingAvatar = false
    @State private var changingPassword = false
    @State private var editingBirthday = false
    @State private var deletingAccount = false
    @State private var avatarError: String?

    var body: some View {
        NavigationStack {
            Form {
                profileSection
                familySection
                privacySection
                serverSection
                sessionSection
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
            .sheet(isPresented: $changingPassword) {
                ChangePasswordView()
            }
            .sheet(isPresented: $editingBirthday) {
                MyBirthdayView()
            }
            .sheet(isPresented: $deletingAccount) {
                DeleteAccountView()
            }
            .sheet(isPresented: Bindable(model).showsStatistics) {
                NavigationStack { StatisticsView() }
            }
            .task {
                await model.load(api: coordinator.api)
            }
            .confirmationDialog(
                "Leave the family?",
                isPresented: Bindable(model).confirmLeave,
                titleVisibility: .visible
            ) {
                Button("Leave Family", role: .destructive) { leave() }
            } message: {
                Text(leaveMessage)
            }
            // A leave dialog open when ownership changes is stale in both
            // halves: who inherits, and what leaving costs. Take it down,
            // re-read, and put it back rather than letting somebody
            // confirm against an answer that has moved
            // (docs/protocol.md, `POST /families/leave`).
            .onChange(of: session.familyOwnerGeneration) {
                guard model.confirmLeave else { return }
                model.confirmLeave = false
                Task {
                    await model.load(api: coordinator.api)
                    model.confirmLeave = true
                }
            }
            .confirmationDialog(
                "Log out?",
                isPresented: Bindable(model).confirmLogout,
                titleVisibility: .visible
            ) {
                Button("Log Out", role: .destructive) {
                    Task { await session.logout() }
                }
            } message: {
                Text("Messages stay on the family server; this device forgets its session.")
            }
            // Replaces a "you're the owner, you can't leave" alert that
            // could no longer happen: `owner_cannot_leave` is retired and
            // no endpoint raises it. An owner who leaves hands the family
            // on and is never refused (docs/protocol.md,
            // `POST /families/leave`).
            .alert(
                "Ownership passed on",
                isPresented: Binding(
                    get: { model.handedOverTo != nil },
                    set: { if !$0 { model.handedOverTo = nil } })
            ) {
                Button("OK", role: .cancel) { dismiss() }
            } message: {
                if let name = model.handedOverTo {
                    Text("\(name) is now the owner of the family.")
                }
            }
        }
    }

    // MARK: - Sections

    /// Presented as a sheet rather than pushed: it is a short, modal
    /// errand with its own Cancel, and it must not be left half-done
    /// behind a back swipe while a save is in flight.

    private var profileSection: some View {
        Section("Profile") {
            if let user = session.currentUser {
                // The identity row the Mac's settings got in its
                // beautification pass: a larger face and a title3 name,
                // so the screen opens on WHO this is rather than on a row.
                HStack(spacing: 14) {
                    InitialsAvatar(
                        title: user.displayName,
                        userID: user.id,
                        avatarVersion: user.avatarVersion,
                        size: 64)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(user.displayName)
                            .font(.title3.weight(.semibold))
                        Text("@\(user.username)")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if uploadingAvatar {
                        ProgressView()
                    }
                }
                .padding(.vertical, 4)
                // The picker hands back an item, not bytes; the transfer
                // and the downscale happen in setAvatar.
                // No `photoLibrary:` argument on purpose. Passing `.shared()`
                // hands the picker the app's own library object, which makes
                // the binary reference PHPhotoLibrary and would eventually
                // want NSPhotoLibraryUsageDescription — for nothing, since
                // nothing here touches PHAsset or an itemIdentifier. Without
                // it the picker runs out of process and returns only the
                // items the person chose, which is all setAvatar needs.
                PhotosPicker(selection: $pickedPhoto, matching: .images) {
                    Label(
                        user.avatarVersion > 0 ? "Change Photo" : "Add Photo",
                        systemImage: "person.crop.circle.badge.plus")
                }
                .disabled(uploadingAvatar)
                if user.avatarVersion > 0 {
                    Button("Remove Photo", role: .destructive) { removeAvatar() }
                        .disabled(uploadingAvatar)
                }
                if let avatarError {
                    Label(avatarError, systemImage: "exclamationmark.circle")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                // Day and month, no year — so it can be shown to the
                // family without publishing an age (protocol.md,
                // "Birthdays"). Laid out like the photo rows above it: the
                // value, then the one button that changes it.
                LabeledContent(
                    "Birthday",
                    value: user.birthday?.formatted() ?? String(localized: "Not set"))
                Button {
                    editingBirthday = true
                } label: {
                    Label(
                        user.birthday == nil ? "Add Birthday…" : "Change Birthday…",
                        systemImage: "birthday.cake")
                }
                Button {
                    changingPassword = true
                } label: {
                    Label("Change Password", systemImage: "key")
                }
            }
        }
        .onChange(of: pickedPhoto) { _, item in
            guard let item else { return }
            setAvatar(item)
        }
    }

    /// Load the picked image, square-crop and downscale it, and upload
    /// the JPEG. The server stores what it is given and never transcodes,
    /// so producing something small and square is this side's job.
    private func setAvatar(_ item: PhotosPickerItem) {
        uploadingAvatar = true
        avatarError = nil
        Task {
            defer {
                uploadingAvatar = false
                pickedPhoto = nil
            }
            // The two failures below are NOT the same thing and must not
            // share a message: one means the library never handed the
            // bytes over (an iCloud photo that isn't downloaded, a
            // revoked pick), the other means we got bytes that aren't a
            // decodable image. Telling them apart is the difference
            // between "try again on wifi" and "pick another photo".
            let data: Data?
            do {
                data = try await item.loadTransferable(type: Data.self)
            } catch {
                AppLog.api.error("Avatar transfer failed: \(String(describing: error))")
                avatarError = "Couldn't load that photo from your library."
                return
            }
            guard let data else {
                avatarError = "Couldn't load that photo from your library."
                return
            }
            guard let jpeg = AvatarImage.squareJPEG(from: data) else {
                AppLog.api.error("Avatar re-encode failed for \(data.count, privacy: .public) bytes")
                avatarError = "That image couldn't be read."
                return
            }
            do {
                let user = try await coordinator.api.uploadAvatar(jpeg: jpeg)
                session.applyProfile(user)
            } catch APIError.unauthorized {
                session.handleUnauthorized()
            } catch {
                AppLog.api.error("Avatar upload failed: \(String(describing: error))")
                avatarError = AvatarFailure.message(for: error, verb: "upload")
            }
        }
    }

    private func removeAvatar() {
        uploadingAvatar = true
        avatarError = nil
        Task {
            defer { uploadingAvatar = false }
            do {
                try await coordinator.api.deleteAvatar()
                if let user = session.currentUser {
                    session.applyProfile(UserDTO(
                        id: user.id,
                        username: user.username,
                        displayName: user.displayName,
                        createdAt: user.createdAt,
                        avatarVersion: 0))
                }
            } catch APIError.unauthorized {
                session.handleUnauthorized()
            } catch {
                AppLog.api.error("Avatar remove failed: \(String(describing: error))")
                avatarError = AvatarFailure.message(for: error, verb: "remove")
            }
        }
    }

    @ViewBuilder
    private var familySection: some View {
        Section {
            if let family = model.family ?? session.family {
                // "Name", under a section already headed "Family".
                LabeledContent("Name", value: family.name)
                // `value:` is a String, not a key — localized by hand.
                LabeledContent(
                    "Join policy",
                    value: family.joinPolicy == "open" ? String(localized: "Open") : String(localized: "Approval"))
                if session.isOwner, let code = model.family?.inviteCode {
                    LabeledContent("Invite code") {
                        Text(code)
                            .font(.body.monospaced())
                            .textSelection(.enabled)
                    }
                    // An explicit Copy beside the selection: on an iPad
                    // with a pointer, selecting text in a list row is a
                    // guess; a menu item is not.
                    .contextMenu {
                        Button {
                            UIPasteboard.general.string = code
                        } label: {
                            Label("Copy", systemImage: "doc.on.doc")
                        }
                    }
                    let server = AppSettings.serverURL?.absoluteString ?? ""
                    ShareLink(
                        item: String(localized: "Join our family on Family Connect! Server: \(server) — invite code: \(code)")
                    ) {
                        Label("Share Invite", systemImage: "square.and.arrow.up")
                    }
                }
                // Everyone gets in: the roster is not owner-gated on the
                // server, only the invite code, policy, requests and
                // removal are — and the screen hides those for members.
                NavigationLink {
                    FamilyManageView(settingsModel: model)
                } label: {
                    Label(
                        session.isOwner ? "Manage Family" : "Family Members",
                        systemImage: "person.2.badge.gearshape")
                }
            }
            Button("Leave Family", role: .destructive) {
                // A fresh read FIRST. The successor is a prediction that
                // any join or leave changes, and no frame announces it —
                // so naming one from the cached copy is how a dialog comes
                // to promise the family to somebody who already left
                // (docs/protocol.md, `GET /families/mine`).
                Task {
                    await model.load(api: coordinator.api)
                    model.confirmLeave = true
                }
            }
        } header: {
            Text("Family")
        } footer: {
            if let error = model.errorText {
                Label(error, systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
    }

    /// The settings that change who the app talks to, so they say so
    /// plainly rather than hiding behind a label.
    private var privacySection: some View {
        Section {
            Toggle("Link Previews", isOn: $linkPreviewsEnabled)
                .onChange(of: linkPreviewsEnabled) { _, newValue in
                    AppSettings.linkPreviewsEnabled = newValue
                }
            Toggle("Map Previews", isOn: $mapPreviewsEnabled)
                .onChange(of: mapPreviewsEnabled) { _, newValue in
                    AppSettings.mapPreviewsEnabled = newValue
                }
            // Guideline 5.1.1(i) requires the privacy policy to be
            // reachable from INSIDE the app, not only from the store
            // listing. A `Link` rather than an in-app browser: the policy
            // is a public page and the system browser is where a reader
            // can check the address bar for themselves.
            Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/privacy.html")!) {
                Label("Privacy Policy", systemImage: "hand.raised")
            }
            Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/support.html")!) {
                Label("Support", systemImage: "questionmark.circle")
            }
        } header: {
            Text("Privacy")
        } footer: {
            Text("Shows a preview under links in messages, and a map on a shared location. Building either asks somebody else for it — the linked website for its title and image, Apple for the map — so they see a request from this device. With maps off, a shared location still shows its pin and opens in Maps when you tap it.")
        }
    }

    // Two sections, so @ViewBuilder rather than a single expression.
    @ViewBuilder
    private var serverSection: some View {
        Section {
            Button {
                model.showsStatistics = true
            } label: {
                Label("Statistics", systemImage: "chart.bar")
            }
        }

        Section("Server") {
            let address = AppSettings.serverURL?.absoluteString ?? "—"
            LabeledContent("Address", value: address)
                .contextMenu {
                    Button {
                        UIPasteboard.general.string = address
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                }
        }
    }

    private var sessionSection: some View {
        Section {
            Button("Log Out", role: .destructive) {
                model.confirmLogout = true
            }
            // Beside Log Out, because that is where somebody looks for
            // "get me out of this app" — and App Store guideline
            // 5.1.1(v) asks for it to be findable, not buried. The sheet
            // is where the explaining happens; this is just the door.
            Button("Delete Account", role: .destructive) {
                deletingAccount = true
            }
        } footer: {
            // verbatim: a product name and two numbers have nothing to
            // translate, and going through the catalogue would make the
            // build number a "string" needing 8 translations.
            Text(verbatim: AppVersion.settingsLine)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 8)
        }
    }

    // MARK: - Actions

    private func leave() {
        model.errorText = nil
        Task {
            do {
                // The roster is captured HERE, before the call: leaving
                // purges it, and the id the server answers with has to be
                // resolved against the names this client still holds.
                let roster = model.members
                let name = try await session.leaveFamily { id in
                    roster.first { $0.id == id }?.displayName
                }
                // Say who it went to, and close on the OK. Nothing to say
                // when nobody inherited — an ordinary member leaving, or
                // the last one, who took the family with them.
                if let name {
                    model.handedOverTo = name
                } else {
                    dismiss()
                }
            } catch {
                model.errorText = String(localized: "Couldn't leave right now. Try again.")
            }
        }
    }

    /// What leaving costs, which is a different sentence for each of the
    /// three people who can read it.
    private var leaveMessage: LocalizedStringKey {
        let base: LocalizedStringKey = "You'll lose access to the family chat and your direct chats. Your history returns if you rejoin."
        guard session.isOwner else { return base }
        guard let successorID = model.nextOwnerUserID,
              let successor = model.members.first(where: { $0.id == successorID })?.displayName
        else {
            // An owner with `next_owner_user_id` absent is the last member
            // standing, and leaving takes the family with them. Nothing
            // returns on a rejoin, because there is nothing to return to.
            return "You're the only member left. Leaving deletes the family and everything in it."
        }
        return "\(successor) becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin."
    }
}

#endif
