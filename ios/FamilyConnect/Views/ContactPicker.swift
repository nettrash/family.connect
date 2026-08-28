//
//  ContactPicker.swift
//  FamilyConnect
//
//  The system contact picker, for linking a family member to a device
//  contact (ContactLinks). CNContactPickerViewController is used on
//  purpose over the Contacts framework proper: it needs NO Contacts
//  permission and prompts for none — the app only ever receives the one
//  contact the person tapped, which is exactly the privacy shape this
//  app keeps everywhere else.
//

#if os(iOS)

import ContactsUI
import SwiftUI

struct ContactPicker: UIViewControllerRepresentable {
    var onPick: (ContactLink) -> Void
    /// The picker dismisses itself either way; the owner of the sheet
    /// clears its item from these, so a cancelled sheet can be opened
    /// again for the same member.
    var onCancel: () -> Void = {}

    func makeUIViewController(context: Context) -> CNContactPickerViewController {
        let picker = CNContactPickerViewController()
        picker.delegate = context.coordinator
        // What the link keeps (ContactLink): the numbers and e-mails the
        // Phone app will hand back when a Family button on this contact is
        // tapped. Displayed on the card too, so the person sees what they
        // are linking.
        picker.displayedPropertyKeys = [CNContactPhoneNumbersKey, CNContactEmailAddressesKey]
        return picker
    }

    func updateUIViewController(_ uiViewController: CNContactPickerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator {
        Coordinator(onPick: onPick, onCancel: onCancel)
    }

    final class Coordinator: NSObject, CNContactPickerDelegate {
        private let onPick: (ContactLink) -> Void
        private let onCancel: () -> Void

        init(onPick: @escaping (ContactLink) -> Void, onCancel: @escaping () -> Void) {
            self.onPick = onPick
            self.onCancel = onCancel
        }

        func contactPickerDidCancel(_ picker: CNContactPickerViewController) {
            onCancel()
        }

        func contactPicker(_ picker: CNContactPickerViewController, didSelect contact: CNContact) {
            let name = CNContactFormatter.string(from: contact, style: .fullName)
                ?? [contact.givenName, contact.familyName].filter { !$0.isEmpty }.joined(separator: " ")
            onPick(ContactLink(
                contactIdentifier: contact.identifier,
                contactName: name,
                phoneNumbers: contact.phoneNumbers.map { $0.value.stringValue },
                emailAddresses: contact.emailAddresses.map { String($0.value) }))
        }
    }
}

#endif
