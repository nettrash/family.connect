//
//  CallIntents.swift
//  FamilyConnect
//
//  The iPhone's side of "call somebody on Family" when the asking is done
//  OUTSIDE the app: SiriKit's call intents. Two doors, one destination.
//
//  The first door is a user activity: when a Family row in the Phone
//  app's Recents is tapped, or "Family voice call" is chosen on a contact
//  card or in Favorites, iOS launches (or foregrounds) the app with an
//  NSUserActivity whose `interaction` carries the start-call intent and
//  the person meant. RootView hands it to `CallRequest.parse`, which
//  reduces it to a CallRequest; ChatListView routes that once the
//  session is active (the pendingPushRoute idiom, so a cold start works).
//
//  The second door is Siri itself — "call Anna on Family": the intent is
//  handed to CallIntentHandler (through the app delegate, iOS 14+ needs
//  no extension for this), which resolves the spoken name against the
//  roster and answers `continueInApp`, and the system then comes through
//  the first door with the resolved person. The handler never places the
//  call: everything that touches the socket happens in the app proper.
//
//  The three intent classes are all handled because the system still
//  delivers the two older ones (INStartAudioCallIntent /
//  INStartVideoCallIntent) from some of its surfaces.
//

#if os(iOS)

import Foundation
import Intents
import os

extension CallRequest {

    /// The `NSUserActivityTypes` this app continues (Info.plist).
    static let activityTypes = ["INStartCallIntent", "INStartAudioCallIntent", "INStartVideoCallIntent"]

    /// The request behind a call activity, or nil for an activity that is
    /// not a call at all.
    static func parse(activity: NSUserActivity) -> CallRequest? {
        guard let intent = activity.interaction?.intent else { return nil }
        if let call = intent as? INStartCallIntent {
            return CallRequest(person: call.contacts?.first, video: call.callCapability == .videoCall)
        }
        // The pre-iOS 13 audio/video intents, which the system STILL
        // delivers from Contacts, Recents and Favorites (see
        // ios-call-provider-gotchas). Matched by class name and read by
        // key-value coding so this file never names the deprecated
        // classes — a deprecation warning here would be a warning about
        // the one thing this branch exists to handle.
        if let audio = Self.legacyAudioIntent, intent.isKind(of: audio) {
            return CallRequest(person: Self.legacyContacts(of: intent)?.first, video: false)
        }
        if let video = Self.legacyVideoIntent, intent.isKind(of: video) {
            return CallRequest(person: Self.legacyContacts(of: intent)?.first, video: true)
        }
        return nil
    }

    private static let legacyAudioIntent: AnyClass? = NSClassFromString("INStartAudioCallIntent")
    private static let legacyVideoIntent: AnyClass? = NSClassFromString("INStartVideoCallIntent")

    /// `contacts` on either legacy intent — an Objective-C property, so
    /// key-value coding reaches it without the Swift symbol.
    private static func legacyContacts(of intent: INIntent) -> [INPerson]? {
        intent.value(forKey: "contacts") as? [INPerson]
    }

    /// From the person the system named. Our own handle is recognised in
    /// either place it can arrive — `customIdentifier` (what
    /// CallIntentHandler sets) or the handle itself (what Recents sends
    /// back); a phone number or e-mail is carried as such, and the
    /// contact identifier is the Favorites / contact-card link.
    init(person: INPerson?, video: Bool) {
        var handle: Handle?
        if let custom = person?.customIdentifier, CallHandle.userID(from: custom) != nil {
            handle = .generic(custom)
        } else if let personHandle = person?.personHandle, let value = personHandle.value, !value.isEmpty {
            switch personHandle.type {
            case .phoneNumber: handle = .phoneNumber(value)
            case .emailAddress: handle = .emailAddress(value)
            default: handle = .generic(value)
            }
        }
        self.init(
            handle: handle,
            contactIdentifier: person?.contactIdentifier,
            contactName: person?.displayName,
            video: video)
    }
}

/// Tell the system a call happened, so Siri's suggestions and the Phone
/// app's association of Family with a person have something to go on
/// (Apple's DTS: "make a donation through INInteraction when the call is
/// started"). The person carries our handle as the custom identifier —
/// which is how a suggestion comes back resolvable — and, when linked,
/// the device contact.
@MainActor
enum CallDonation {
    static func donate(peerUserID: Int64, peerName: String, video: Bool, contactIdentifier: String?) {
        let handle = CallHandle.value(userID: peerUserID)
        let person = INPerson(
            personHandle: INPersonHandle(value: handle, type: .unknown),
            nameComponents: nil,
            displayName: peerName,
            image: nil,
            contactIdentifier: contactIdentifier,
            customIdentifier: handle)
        let intent = INStartCallIntent(
            callRecordFilter: nil,
            callRecordToCallBack: nil,
            audioRoute: .unknown,
            destinationType: .normal,
            contacts: [person],
            callCapability: video ? .videoCall : .audioCall)
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .outgoing
        // The handle, so one person's donations can be withdrawn on their
        // own when they are blocked. Without a group there is no selective
        // delete — only `deleteAll`, which would throw away everybody's.
        interaction.groupIdentifier = handle
        interaction.donate { error in
            if let error {
                AppLog.call.info("Call donation refused: \(String(describing: error))")
            }
        }
    }

    /// Withdraw the suggestions this app made about one person.
    ///
    /// A donation is the app's own statement to the system, and it goes on
    /// being made in the app's name — in Siri's suggestions, in the Phone
    /// app's sense of who Family can call — long after it stops being
    /// true. Blocking somebody is exactly when it stops being true, so the
    /// statement is retracted (docs/protocol.md, "Calls").
    ///
    /// Device hygiene, not part of the silence: it happens on the
    /// blocker's own device and changes nothing anybody else can observe.
    /// Nothing puts these back on unblock — they were suggestions, not
    /// history, and the call RECORDS in the chat are untouched.
    static func withdraw(peerUserID: Int64) {
        INInteraction.delete(with: CallHandle.value(userID: peerUserID)) { error in
            if let error {
                AppLog.call.info("Call donation withdrawal refused: \(String(describing: error))")
            }
        }
    }
}

/// Siri's questions about a call, answered from the roster.
@MainActor
final class CallIntentHandler: NSObject, INStartCallIntentHandling {

    /// The active roster (not me, not left, not deleted): who Siri may
    /// be asked to call. Wired by the composition root.
    var roster: () -> [CallRequestRouter.Candidate] = { [] }

    nonisolated func resolveContacts(
        for intent: INStartCallIntent,
        with completion: @escaping ([INStartCallContactResolutionResult]) -> Void
    ) {
        guard let person = intent.contacts?.first else {
            completion([.needsValue()])
            return
        }
        Task { @MainActor in
            completion([self.resolve(person)])
        }
    }

    private func resolve(_ person: INPerson) -> INStartCallContactResolutionResult {
        let roster = roster()
        // Already one of ours (a redial, or a previous resolution).
        if let custom = person.customIdentifier, let id = CallHandle.userID(from: custom),
           let member = roster.first(where: { $0.userID == id }) {
            return .success(with: Self.person(for: member))
        }
        switch CallRequestRouter.match(name: person.displayName, in: roster) {
        case .one(let member):
            return .success(with: Self.person(for: member))
        case .several(let members):
            return .disambiguation(with: members.map(Self.person(for:)))
        case .none:
            return .unsupported(forReason: .noContactFound)
        }
    }

    /// The member as Siri will name them back to us: our handle in both
    /// the handle and the custom identifier, the display name for Siri to
    /// say. No contact identifier — Siri is matching the roster, not the
    /// address book.
    private static func person(for member: CallRequestRouter.Candidate) -> INPerson {
        let handle = CallHandle.value(userID: member.userID)
        return INPerson(
            personHandle: INPersonHandle(value: handle, type: .unknown),
            nameComponents: nil,
            displayName: member.name,
            image: nil,
            contactIdentifier: nil,
            customIdentifier: handle)
    }

    nonisolated func resolveCallCapability(
        for intent: INStartCallIntent,
        with completion: @escaping (INStartCallCallCapabilityResolutionResult) -> Void
    ) {
        completion(.success(with: intent.callCapability == .videoCall ? .videoCall : .audioCall))
    }

    nonisolated func resolveDestinationType(
        for intent: INStartCallIntent,
        with completion: @escaping (INCallDestinationTypeResolutionResult) -> Void
    ) {
        completion(.success(with: .normal))
    }

    /// Never handled here: the app places the call, with the socket, the
    /// microphone prompt and CallKit all where they already live.
    nonisolated func handle(
        intent: INStartCallIntent,
        completion: @escaping (INStartCallIntentResponse) -> Void
    ) {
        completion(INStartCallIntentResponse(code: .continueInApp, userActivity: nil))
    }
}

#endif
