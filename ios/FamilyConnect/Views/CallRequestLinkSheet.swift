//
//  CallRequestLinkSheet.swift
//  FamilyConnect
//
//  "Which family member is this?" — shown when the system asks Family to
//  call a device contact nobody on this phone has linked to a member yet
//  (a Favorites entry made before linking, a contact card button), or a
//  name two members share (a Recents row of an unlinked member). With a
//  contact, the pick is kept (ContactLinks) and the call goes ahead;
//  without one it is this call's choice. The same list as NewChatView,
//  for the same reason: not me, not left, not deleted.
//

#if os(iOS)

import SwiftData
import SwiftUI

struct CallRequestLinkSheet: View {
    /// nil when there is no device contact to remember the answer for.
    let contactIdentifier: String?
    let contactName: String?
    /// The number or e-mail the request arrived with, kept on the link so
    /// the same button matches next time even when the system sends no
    /// contact identifier (the sheet has no Contacts access to fetch the
    /// rest — by design).
    var handle: CallRequest.Handle?
    /// The member chosen — the link is already stored by then.
    var onPick: (Int64) -> Void

    @Environment(\.dismiss) private var dismiss
    @Query(filter: #Predicate<MemberEntity> { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted },
           sort: [SortDescriptor(\MemberEntity.displayName)])
    private var members: [MemberEntity]

    private var title: String {
        if let contactName, !contactName.isEmpty {
            return String(localized: "Who is \(contactName)?", comment: "Title of the sheet asking which family member a device contact is; the argument is the contact's name.")
        }
        return String(localized: "Who is this?", comment: "Title of the sheet asking which family member a device contact is, when the contact has no name.")
    }

    var body: some View {
        NavigationStack {
            Group {
                if members.isEmpty {
                    ContentUnavailableView(
                        "No one else yet",
                        systemImage: "person.2",
                        description: Text("Direct chats become available once another member joins the family."))
                } else {
                    List {
                        Section {
                            ForEach(members) { member in
                                Button {
                                    if let contactIdentifier {
                                        var phones: [String] = []
                                        var emails: [String] = []
                                        switch handle {
                                        case .phoneNumber(let number)?: phones = [number]
                                        case .emailAddress(let email)?: emails = [email]
                                        default: break
                                        }
                                        ContactLinks.shared.link(
                                            userID: member.userID,
                                            to: ContactLink(
                                                contactIdentifier: contactIdentifier, contactName: contactName ?? "",
                                                phoneNumbers: phones, emailAddresses: emails))
                                    }
                                    dismiss()
                                    onPick(member.userID)
                                } label: {
                                    HStack(spacing: 12) {
                                        InitialsAvatar(
                                            title: member.displayName,
                                            userID: member.userID,
                                            avatarVersion: member.avatarVersion)
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(member.displayName)
                                                .foregroundStyle(.primary)
                                            Text("@\(member.username)")
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        } footer: {
                            if contactIdentifier != nil {
                                Text("Family remembers the link on this device only; nothing about the contact is sent anywhere.")
                            }
                        }
                    }
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                }
            }
        }
    }
}

#endif
