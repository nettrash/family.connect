//
//  AppSettings.swift
//  FamilyConnect
//
//  Typed accessors over the app's UserDefaults. Keys are versioned
//  ("v1.…") so a future schema change can migrate by key prefix instead
//  of guessing what an unversioned value meant. Nothing sensitive lives
//  here — the session token is in the Keychain (KeychainStore); this is
//  routing/bookkeeping state only.
//
//  Declared in PrivacyInfo.xcprivacy under accessed-API reason CA92.1
//  (app accesses only its own defaults).
//

import Foundation

nonisolated enum AppSettings {

    private static var defaults: UserDefaults { .standard }

    private enum Key {
        static let serverURL = "v1.serverURL"
        static let currentUserID = "v1.currentUserID"
        static let joinPending = "v1.joinPending"
        static let pushToken = "v1.push.token"
        static let pushDeviceID = "v1.push.deviceID"
        /// The PushKit VoIP token the server has confirmed, beside the
        /// APNs pair above — an incoming call is delivered to this one.
        static let voipToken = "v1.push.voipToken"
        /// Stores the DISABLED flag, so a missing key reads as "on".
        static let linkPreviewsDisabled = "v1.linkPreviewsDisabled"
        /// Stored INVERTED, exactly like the link-preview key above and for
        /// the same reason: a missing key must read as "on", and a logout
        /// (or any 401) must not silently turn third-party traffic back on
        /// for somebody who opted out of it.
        static let mapPreviewsDisabled = "v1.mapPreviewsDisabled"
        /// The board catch-up cursor: the highest board_seq this device
        /// has APPLIED. Local-only and account-scoped, so it is wiped with
        /// the session — a different family's board must never be caught
        /// up from another's cursor.
        static let boardCursor = "v1.board.cursor"
        static let boardSeenNoteID = "v1.board.seenNoteId"
        /// The badge's real mark: the highest `content_seq` this device has
        /// SHOWN. Separate key rather than a reused one, because the two
        /// numbers come from different spaces — a note id and a board seq —
        /// and a device that upgrades has a meaningful value for the old
        /// one and none for the new (BoardBadge.contentMarkSeed).
        static let boardSeenContentSeq = "v1.board.seenContentSeq"
        /// The assistant, as `GET /families/mine` last reported it. Two
        /// jobs at once: naming its messages in the family chat, where its
        /// reserved account is deliberately absent from the roster, and
        /// telling the composer whether to offer `@ai` at all.
        static let assistantUserID = "v1.assistant.userId"
        static let assistantName = "v1.assistant.name"
        /// What this SERVER's assistant can do beyond words, as
        /// `GET /families/mine` last reported it (protocol.md, "Pictures").
        /// Stored the plain way round — a missing key reads as "cannot" —
        /// because that is both the honest answer for a server that
        /// predates the feature and the one that offers nothing.
        static let assistantVision = "v1.assistant.vision"
        static let assistantImages = "v1.assistant.images"
        /// The picture token as the server spells it. Held so the client
        /// can be certain the server means the same five characters by it
        /// before offering an affordance built on its own copy.
        static let assistantDraw = "v1.assistant.draw"
        /// Pre-push installs stored a "registered once, token null"
        /// boolean under this key; superseded by the pair above and only
        /// referenced by wipe() so upgraded installs shed it.
        static let legacyDeviceRegistered = "v1.deviceRegistered"
    }

    /// The server URL compiled into this build, or nil for the generic
    /// build. This is the "predefined default server" mechanism for the
    /// App Store build: the `Release-nettrash` configuration sets the
    /// user-defined build setting `FC_DEFAULT_SERVER_URL`, Info.plist
    /// carries it as `FCDefaultServerURL = $(FC_DEFAULT_SERVER_URL)`, and
    /// AppSession.bootstrap adopts it when no server URL is stored yet —
    /// so store users land straight on Register/Login instead of the
    /// server-setup screen. Debug/Release leave the setting empty, which
    /// this accessor reports as nil (first run keeps asking for a URL).
    ///
    /// The raw plist string is trimmed and run through the same
    /// `ServerURLNormalizer` the setup screen uses, so the `serverURL`
    /// invariant (scheme present, no trailing slash) holds no matter how
    /// the build setting was spelled; anything the normalizer rejects is
    /// treated as "no default" rather than adopted broken.
    static var defaultServerURL: URL? {
        guard let raw = Bundle.main.object(forInfoDictionaryKey: "FCDefaultServerURL") as? String else {
            return nil
        }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        switch ServerURLNormalizer.normalize(trimmed) {
        case .ok(let url), .okInsecureLocal(let url):
            return url
        case .invalid:
            return nil
        }
    }

    /// The user-entered server base URL (normalized: scheme present,
    /// no trailing slash). nil until the setup screen confirms one.
    static var serverURL: URL? {
        get { defaults.string(forKey: Key.serverURL).flatMap(URL.init(string:)) }
        set {
            if let newValue {
                defaults.set(newValue.absoluteString, forKey: Key.serverURL)
            } else {
                defaults.removeObject(forKey: Key.serverURL)
            }
        }
    }

    /// The signed-in user's server id; lets the sync layer attribute
    /// "mine" without a /me round-trip.
    static var currentUserID: Int64? {
        get {
            let value = defaults.object(forKey: Key.currentUserID) as? NSNumber
            return value?.int64Value
        }
        set {
            if let newValue {
                defaults.set(NSNumber(value: newValue), forKey: Key.currentUserID)
            } else {
                defaults.removeObject(forKey: Key.currentUserID)
        defaults.removeObject(forKey: Key.assistantUserID)
        defaults.removeObject(forKey: Key.assistantName)
            }
        }
    }

    /// True while a join request is outstanding. Persisted so a relaunch
    /// can still detect "was waiting, now neither family nor request ⇒
    /// declined" (the protocol's only rejection signal).
    static var joinPending: Bool {
        get { defaults.bool(forKey: Key.joinPending) }
        set { defaults.set(newValue, forKey: Key.joinPending) }
    }

    /// Whether a message's first web link gets a preview card. On by
    /// default, but switchable because building one means THIS device
    /// requests the linked page — the only routine traffic the app sends
    /// anywhere but the family's own server. Stored inverted so the
    /// absent key (`.bool` → false) reads as on.
    static var linkPreviewsEnabled: Bool {
        get { !defaults.bool(forKey: Key.linkPreviewsDisabled) }
        set { defaults.set(!newValue, forKey: Key.linkPreviewsDisabled) }
    }

    /// Whether a shared location draws a map in the bubble. The same trade
    /// as link previews and therefore the same switch shape: drawing tiles
    /// means asking Apple for them, with a coordinate a family member
    /// deliberately sent. Off, the bubble still shows the pin, the label
    /// and a way into the system map app — which is a hand-off the reader
    /// chooses rather than a request the app makes on its own.
    /// Stored inverted so the absent key reads as on.
    static var mapPreviewsEnabled: Bool {
        get { !defaults.bool(forKey: Key.mapPreviewsDisabled) }
        set { defaults.set(!newValue, forKey: Key.mapPreviewsDisabled) }
    }

    /// The APNs token (lowercase hex) most recently accepted by
    /// POST /devices. Paired with `pushDeviceID` below — PushRegistrar
    /// sets both together after a 2xx, so "token differs from stored" is
    /// exactly the re-POST condition. The token itself is an opaque
    /// routing handle, not a secret (it is useless without the server's
    /// APNs key), so defaults — not the keychain — is the right home.
    static var pushToken: String? {
        get { defaults.string(forKey: Key.pushToken) }
        set {
            if let newValue {
                defaults.set(newValue, forKey: Key.pushToken)
            } else {
                defaults.removeObject(forKey: Key.pushToken)
            }
        }
    }

    /// The PushKit VoIP token (lowercase hex) most recently accepted by
    /// POST /devices, or nil when none has been. iOS only in practice — a
    /// Mac never has one. Same home as `pushToken`, for the same reason.
    static var voipToken: String? {
        get { defaults.string(forKey: Key.voipToken) }
        set {
            if let newValue {
                defaults.set(newValue, forKey: Key.voipToken)
            } else {
                defaults.removeObject(forKey: Key.voipToken)
            }
        }
    }

    /// The server's device row id for `pushToken` — what
    /// DELETE /devices/{id} takes on logout.
    static var pushDeviceID: Int64? {
        get {
            let value = defaults.object(forKey: Key.pushDeviceID) as? NSNumber
            return value?.int64Value
        }
        set {
            if let newValue {
                defaults.set(NSNumber(value: newValue), forKey: Key.pushDeviceID)
            } else {
                defaults.removeObject(forKey: Key.pushDeviceID)
            }
        }
    }

    /// Remove everything this type owns; `keepServerURL` preserves the
    /// server row (logout keeps it; server change does not).
    /// Highest board_seq applied on this device; 0 = nothing yet, which is
    /// what makes the first open do a full board read rather than replay
    /// every note that ever existed.
    static var boardCursor: Int64 {
        get { Int64(defaults.integer(forKey: Key.boardCursor)) }
        set { defaults.set(Int(newValue), forKey: Key.boardCursor) }
    }

    /// Highest note id the user has actually BEEN SHOWN, for the badge.
    ///
    /// Deliberately not `boardCursor`. That is a SYNC cursor and advances
    /// whenever a change is applied — including a background resync — so
    /// using it here would clear the badge for someone who never opened the
    /// board. This one moves only when the board is on screen. Note ids are
    /// server-assigned and monotonic, so "created since you last looked" is
    /// just "id greater than this".
    static var boardSeenNoteID: Int64 {
        get { Int64(defaults.integer(forKey: Key.boardSeenNoteID)) }
        set { defaults.set(Int(newValue), forKey: Key.boardSeenNoteID) }
    }

    /// Highest `content_seq` the user has actually BEEN SHOWN — what the
    /// board badge counts (docs/protocol.md, "Board").
    ///
    /// `boardSeenNoteID` above is kept beside it, and still used, for notes
    /// that carry no content seq: rows cached before the field existed, and
    /// notes from a server that predates it. BoardBadge holds the rule; this
    /// is only where the two numbers live.
    static var boardSeenContentSeq: Int64 {
        get { Int64(defaults.integer(forKey: Key.boardSeenContentSeq)) }
        set { defaults.set(Int(newValue), forKey: Key.boardSeenContentSeq) }
    }

    /// The pair, read and written together — a badge that used one mark
    /// from before an update and one from after would be neither rule.
    static var boardMarks: BoardBadge.Marks {
        get {
            BoardBadge.Marks(
                seenNoteID: boardSeenNoteID, seenContentSeq: boardSeenContentSeq)
        }
        set {
            boardSeenNoteID = newValue.seenNoteID
            boardSeenContentSeq = newValue.seenContentSeq
        }
    }

    /// The assistant's reserved account id, or nil when the server has no
    /// assistant configured.
    ///
    /// Absent means BOTH "there is nobody to name" and "do not offer the
    /// mention": a composer that offered `@ai` against a server without an
    /// assistant would offer an affordance that silently does nothing
    /// (docs/protocol.md, "Mentioning the assistant in the family chat").
    static var assistantUserID: Int64? {
        get {
            let stored = defaults.object(forKey: Key.assistantUserID) as? Int
            return stored.map(Int64.init)
        }
        set {
            if let newValue {
                defaults.set(Int(newValue), forKey: Key.assistantUserID)
            } else {
                defaults.removeObject(forKey: Key.assistantUserID)
            }
        }
    }

    /// What to call it. Server-configured, so it is whatever the family's
    /// own server says rather than a string compiled in here.
    static var assistantName: String? {
        get { defaults.string(forKey: Key.assistantName) }
        set {
            if let newValue {
                defaults.set(newValue, forKey: Key.assistantName)
            } else {
                defaults.removeObject(forKey: Key.assistantName)
            }
        }
    }

    /// Whether this server has a deployment that can LOOK at a picture.
    ///
    /// Half of the vision gate and only half: the family's own `ai_vision`
    /// is the other, and a client offers to attach a picture in an `ai`
    /// chat only when BOTH are true (protocol.md, "Pictures"). False here
    /// means the surface is absent rather than disabled — a server without
    /// the deployment configured must show no surface at all, not one that
    /// lies about what would happen. Since #56 the same pair also decides
    /// whether a photo on an `@ai` message, or on the message it replies
    /// to, travels with the mention; that needs no surface of its own.
    static var assistantVision: Bool {
        get { defaults.bool(forKey: Key.assistantVision) }
        set { defaults.set(newValue, forKey: Key.assistantVision) }
    }

    /// Whether this server can GENERATE one. The whole of the `/draw`
    /// capability check: generation has no family switch, because what
    /// leaves on such a request is the words after the token and nothing
    /// else. Since #56 it also means the assistant may draw UNASKED, in
    /// answer to an ordinary question — nothing here changes for that: the
    /// reply is the picture message `/draw` already produces (protocol.md,
    /// "Drawing without being told to").
    static var assistantImages: Bool {
        get { defaults.bool(forKey: Key.assistantImages) }
        set { defaults.set(newValue, forKey: Key.assistantImages) }
    }

    /// The picture token the server named, or nil when it named none.
    static var assistantDraw: String? {
        get { defaults.string(forKey: Key.assistantDraw) }
        set {
            if let newValue {
                defaults.set(newValue, forKey: Key.assistantDraw)
            } else {
                defaults.removeObject(forKey: Key.assistantDraw)
            }
        }
    }

    /// May this client offer the `/draw` affordance?
    ///
    /// The server's answer AND a spelling check. `assistant.draw` exists so
    /// a client can be certain the server means the same five characters
    /// its own grammar does; a server that named a different token is one
    /// this build cannot compose for, and offering a button that types the
    /// wrong thing is exactly the "affordance that silently does nothing"
    /// the capability flags exist to prevent. A server that named NO token
    /// is a server that predates pictures, where `images` is false anyway.
    static var offersPictureRequests: Bool {
        AssistantSurfaces.offersPictureRequests(
            serverCanDraw: assistantImages, serverToken: assistantDraw)
    }

    static func wipe(keepServerURL: Bool) {
        if !keepServerURL { defaults.removeObject(forKey: Key.serverURL) }
        defaults.removeObject(forKey: Key.currentUserID)
        defaults.removeObject(forKey: Key.assistantUserID)
        defaults.removeObject(forKey: Key.assistantName)
        defaults.removeObject(forKey: Key.assistantVision)
        defaults.removeObject(forKey: Key.assistantImages)
        defaults.removeObject(forKey: Key.assistantDraw)
        defaults.removeObject(forKey: Key.joinPending)
        defaults.removeObject(forKey: Key.pushToken)
        defaults.removeObject(forKey: Key.pushDeviceID)
        defaults.removeObject(forKey: Key.voipToken)
        defaults.removeObject(forKey: Key.legacyDeviceRegistered)
        defaults.removeObject(forKey: Key.boardCursor)
        defaults.removeObject(forKey: Key.boardSeenNoteID)
        defaults.removeObject(forKey: Key.boardSeenContentSeq)
        // Member ↔ contact links name user ids of THIS server's family.
        defaults.removeObject(forKey: ContactLinks.key)
    }
}
