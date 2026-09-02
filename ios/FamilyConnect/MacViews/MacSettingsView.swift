//
//  MacSettingsView.swift
//  FamilyConnect
//
//  Settings on the Mac: profile, family, server, session.
//
//  THE Settings scene (FamilyConnectApp), which is what ⌘, and the App
//  menu's "Settings…" open — not the sheet this used to be. It held that
//  shape because most of what is here is account state (who you are, which
//  family, which server) rather than preferences, and a Mac keeps account
//  state in a window it can reach from anywhere; a sheet on the main window
//  was neither. The two standard doors were simply missing, and pressing ⌘,
//  in a Mac app and having nothing happen reads as a broken app.
//
//  The toolbar's gear survives the move and now calls `openSettings()`, so
//  there is still exactly ONE panel: a Settings scene is a singleton, so the
//  gear raises the window the menu item opens rather than making a second
//  copy of this view. That matters beyond tidiness — `mapPreviewsEnabled`
//  and `linkPreviewsEnabled` below are @State MIRRORS of non-observable
//  defaults, so two live copies would show each other's toggles stale.
//
//  There is no Done button any more. A window is closed the way every Mac
//  window is closed (the red button, ⌘W); a Done button that dismissed a
//  sheet is, in a window, a second Close that also steals Return.
//

#if os(macOS)

import SwiftUI

struct MacSettingsView: View {
    @Environment(AppSession.self) private var session
    @Environment(ChatSyncCoordinator.self) private var coordinator

    @State private var changingPassword = false
    @State private var editingBirthday = false
    @State private var showingStatistics = false
    @State private var confirmLogout = false
    @State private var deletingAccount = false
    /// Mirrors AppSettings — defaults are not observable, so the view
    /// holds its own copy and writes through on change.
    @State private var mapPreviewsEnabled = AppSettings.mapPreviewsEnabled
    @State private var linkPreviewsEnabled = AppSettings.linkPreviewsEnabled

    /// Who you are, at the top, with a face — the rows underneath are then
    /// only the things you can DO, which is what a settings sheet is for.
    private var identityHeader: some View {
        HStack(spacing: 12) {
            InitialsAvatar(
                title: session.currentUser?.displayName ?? "?",
                userID: session.currentUser?.id,
                avatarVersion: session.currentUser?.avatarVersion ?? 0,
                size: 56)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.currentUser?.displayName ?? "—")
                    .font(.title3.weight(.semibold))
                if let user = session.currentUser {
                    Text("@\(user.username)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                if session.isOwner {
                    Text("Owner")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tint)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(.tint.opacity(0.15), in: Capsule())
                        .padding(.top, 2)
                }
            }
            Spacer()
        }
        .padding(16)
    }

    /// The birthday as the family sees it, or the plain fact that there
    /// isn't one.
    private var birthdayText: String {
        session.currentUser?.birthday?.formatted() ?? String(localized: "Not set")
    }

    /// The panel itself: who you are, then everything you can do.
    private var panel: some View {
        VStack(alignment: .leading, spacing: 0) {
            identityHeader
            Divider()
            Form {
                Section("Profile") {
                    // Day and month, no year — so it can be shown to the
                    // family without publishing an age (protocol.md,
                    // "Birthdays").
                    LabeledContent("Birthday", value: birthdayText)
                    Button(
                        session.currentUser?.birthday == nil
                            ? "Add Birthday…" : "Change Birthday…"
                    ) { editingBirthday = true }
                    Button("Change Password…") { changingPassword = true }
                }
                Section("Family") {
                    LabeledContent("Family", value: session.family?.name ?? "—")
                }
                Section {
                    Button("Statistics…") { showingStatistics = true }
                }
                // The Mac's only setting that changes who the app talks
                // to, so it says so plainly. Link previews are not drawn
                // here at all, so there is nothing to switch for them; a
                // map on a shared location IS drawn, and drawing one asks
                // Apple for tiles.
                Section {
                    Toggle("Link Previews", isOn: $linkPreviewsEnabled)
                        .onChange(of: linkPreviewsEnabled) { _, newValue in
                            AppSettings.linkPreviewsEnabled = newValue
                        }
                    Toggle("Map Previews", isOn: $mapPreviewsEnabled)
                        .onChange(of: mapPreviewsEnabled) { _, newValue in
                            AppSettings.mapPreviewsEnabled = newValue
                        }
                    // Guideline 5.1.1(i) wants the policy reachable from
                    // inside the app, and the Mac app is its own listing
                    // with its own review — so it needs its own link.
                    Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/privacy.html")!) {
                        Label("Privacy Policy", systemImage: "hand.raised")
                    }
                    Link(destination: URL(string: "https://nettrash.me/appstore/familyconnect/support.html")!) {
                        Label("Support", systemImage: "questionmark.circle")
                    }
                } header: {
                    Text("Privacy")
                } footer: {
                    Text("Shows a preview under links in messages, and a map on a shared location. Building either asks somebody else for it — the linked website for its title and image, Apple for the map — so they see a request from this Mac. With maps off, a shared location still shows its pin and opens in Maps when you click it.")
                }

                Section("Server") {
                    LabeledContent(
                        "Address",
                        value: AppSettings.serverURL?.absoluteString ?? "—")
                }
                Section {
                    Button("Log Out", role: .destructive) { confirmLogout = true }
                    // The shared sheet does the explaining (see
                    // DeleteAccountView); this is the door, next to Log
                    // Out where somebody looks for it.
                    Button("Delete Account…", role: .destructive) { deletingAccount = true }
                } footer: {
                    Text(verbatim: AppVersion.settingsLine)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.top, 8)
                }
            }
            .formStyle(.grouped)
        }
    }

    var body: some View {
        Group {
            // ⌘, is a GLOBAL shortcut: unlike the toolbar's gear, which only
            // exists once MacChatView is on screen, the App menu offers it at
            // the server-setup and sign-in screens too. Every row below reads
            // `session.currentUser` or acts on a session — "Log Out" and
            // "Delete Account…" most of all — so signed out this drew a panel
            // of em-dashes with two destructive buttons under them. Say what
            // is missing instead.
            if session.phase == .active {
                panel
            } else {
                // ONE new string, not a title and a subtitle: this app is
                // localized into nine languages and every sentence added
                // here is nine sentences somebody has to write.
                ContentUnavailableView(
                    "Sign in to see your settings.",
                    systemImage: "person.crop.circle.badge.questionmark")
            }
        }
        // Sized for a WINDOW, not for the sheet this was. 460x530 is the
        // same pair the sheet was grown to and for the same reason — 460 is
        // the width the grouped Form's longest row needs without wrapping,
        // 530 the height that puts "Delete Account…" on screen without
        // scrolling — but it is now a STARTING size rather than the whole
        // story, which is exactly what the old frame's comment said a sheet
        // could not offer.
        //
        // What actually opens the window at it is `.defaultSize` on the
        // scene: a Settings scene ignores the ideal below and opens at the
        // system's own default, measured here as 882x444 — nearly twice as
        // wide as the Form wants and too short for its last section, which
        // is the one holding Delete Account. The ideal stays as the backstop
        // for a macOS that ever honours it instead.
        //
        // The minimum is deliberately BELOW that: at 420 high the last
        // section scrolls, and somebody who has dragged the window that
        // small has chosen to scroll. `.windowResizability(.contentMinSize)`
        // on the scene is what lets them drag it there at all.
        .frame(minWidth: 460, idealWidth: 460, minHeight: 420, idealHeight: 530)
        .background(Color.appGroupedBackground)
        .sheet(isPresented: $changingPassword) {
            ChangePasswordView()
                .frame(width: 420)
        }
        .sheet(isPresented: $editingBirthday) {
            MyBirthdayView()
                .frame(width: 420)
        }
        .sheet(isPresented: $showingStatistics) {
            StatisticsView()
        }
        .sheet(isPresented: $deletingAccount) {
            DeleteAccountView()
                .frame(width: 460)
        }
        .confirmationDialog(
            "Log out?",
            isPresented: $confirmLogout,
            titleVisibility: .visible
        ) {
            Button("Log Out", role: .destructive) {
                Task { await session.logout() }
            }
        } message: {
            Text("Local messages are removed from this Mac.")
        }
    }
}

#endif
