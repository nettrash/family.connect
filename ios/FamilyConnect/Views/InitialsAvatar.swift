//
//  InitialsAvatar.swift
//  FamilyConnect
//
//  A person's picture, or their initials when they have none.
//
//  Shared by both platforms and by half a dozen screens, so it lives on
//  its own rather than inside the chat list that happened to need it
//  first. Platform-free: it resolves through AvatarStore, which deals in
//  CGImage (see PlatformImage).
//

import SwiftUI

/// Circle with the chat title's initials — the classic no-photo avatar.
struct InitialsAvatar: View {
    let title: String
    var isFamily = false
    /// Who this circle stands for. Given both, the profile picture
    /// replaces the initials once it has been fetched; without them (or
    /// before it lands) the initials are what shows, so a row never
    /// waits on the network to render.
    var userID: Int64?
    var avatarVersion: Int64 = 0
    var size: CGFloat = 44

    @Environment(AvatarStore.self) private var avatars

    private var initials: String {
        let words = title.split(separator: " ").prefix(2)
        let letters = words.compactMap { $0.first.map(String.init) }
        return letters.isEmpty ? "?" : letters.joined().uppercased()
    }

    private var picture: Image? {
        guard let userID, avatarVersion > 0 else { return nil }
        return avatars.image(userID: userID, version: avatarVersion)
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(.tint.opacity(0.2))
            if let picture {
                picture
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else if isFamily {
                Image(systemName: "house.fill")
                    .font(.system(size: size * 0.41))
                    .foregroundStyle(.tint)
            } else {
                Text(initials)
                    .font(.system(size: size * 0.36, weight: .semibold))
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
    }
}
