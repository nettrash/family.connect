//
//  APIClient.swift
//  FamilyConnect
//
//  The whole REST surface of the app, one typed method per endpoint in
//  docs/protocol.md. An actor so the mutable (serverURL, token) pair is
//  race-free between the MainActor UI, the sync coordinator and the
//  socket's resync calls.
//
//  Conventions (following Geo's SpaceWeatherService):
//    - `URLSession` injected as the ONLY unit-test seam; the app passes
//      nothing and gets `.shared`, tests pass a session backed by
//      StubURLProtocol.
//    - 15 s per-request timeout — generous for a LAN box, short enough
//      that a dead server doesn't hang a UI await.
//    - ONE retry, on GETs only, for transient statuses (429 / 5xx),
//      honouring `Retry-After` (delta-seconds, capped). POSTs are never
//      auto-retried here: message sending has its own idempotent retry
//      pipeline keyed on client_msg_id, and blind POST retries elsewhere
//      (register, join) could surprise the user.
//    - Every non-2xx is mapped to a typed `APIError` carrying the
//      server's stable machine `code` where one was parseable, so views
//      can switch on codes ("username_taken", "owner_cannot_leave")
//      instead of string-matching messages.
//

import Foundation
import os

nonisolated enum APIError: Error, Equatable {
    /// No server URL configured yet — a programming error surfaced softly.
    case notConfigured
    /// The request never produced an HTTP response (DNS, refused, timeout).
    case transport(URLError)
    /// 401 — the session is gone (or credentials were wrong on /auth/login).
    case unauthorized
    /// 403 with the server's machine code (e.g. "not_family_owner").
    case forbidden(code: String?)
    /// 404 with the server's machine code (e.g. "invalid_invite_code").
    case notFound(code: String?)
    /// 413 — the body was refused as too large. Its own case because a
    /// proxy in front of the server answers it with an HTML page, not the
    /// protocol's error shape, so the code that would say WHY is absent
    /// exactly when the user most needs to be told.
    case payloadTooLarge
    /// Any other 4xx — validation and state conflicts (409 "username_taken",
    /// "owner_cannot_leave", 400 "validation", …). Carries the human
    /// message for inline display.
    case conflict(code: String?, message: String?)
    /// 5xx after the retry budget is spent.
    case server(status: Int, message: String?)
    /// The status was fine but the body did not decode as documented.
    case decoding
}

actor APIClient {

    private var serverURL: URL?
    private(set) var token: String?
    private let session: URLSession

    /// Per-request timeout. See file header.
    private let timeout: TimeInterval = 15
    /// Uploads get their own, far longer budget: 100 MB over a phone's
    /// uplink is minutes, not seconds, and the 15 s that suits a JSON call
    /// would cancel every video.
    private let uploadTimeout: TimeInterval = 600

    /// `session` is the only test seam — see file header.
    init(serverURL: URL?, session: URLSession = .shared) {
        self.serverURL = serverURL
        self.session = session
    }

    /// (Re)point the client. Passing `token: nil` drops authentication
    /// (used on logout / unauthorized purge).
    func configure(serverURL: URL?, token: String?) {
        self.serverURL = serverURL
        self.token = token
    }

    func setToken(_ token: String?) {
        self.token = token
    }

    var currentServerURL: URL? { serverURL }

    // MARK: - Probe

    /// True when `serverURL` looks like a Family Connect server: an
    /// unauthenticated GET /me must answer 401 *with the documented error
    /// body*. A random web server also 401s or 404s, but it won't emit
    /// `{"error":{"code":…}}` — parsing the body is what tells ours apart.
    func probe(serverURL: URL) async -> Bool {
        guard let url = Self.endpointURL(base: serverURL, path: "/me") else { return false }
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        do {
            let (data, response) = try await session.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 401 else { return false }
            return (try? APICoding.decoder().decode(APIErrorBody.self, from: data)) != nil
        } catch {
            return false
        }
    }

    // MARK: - Auth

    private struct RegisterRequest: Encodable {
        let username: String
        let displayName: String
        let password: String
        enum CodingKeys: String, CodingKey {
            case username
            case displayName = "display_name"
            case password
        }
    }

    func register(username: String, displayName: String, password: String) async throws -> AuthResponse {
        try await request("POST", "/auth/register",
                          body: RegisterRequest(username: username, displayName: displayName, password: password))
    }

    private struct LoginRequest: Encodable {
        let username: String
        let password: String
    }

    func login(username: String, password: String) async throws -> AuthResponse {
        try await request("POST", "/auth/login", body: LoginRequest(username: username, password: password))
    }

    func logout() async throws {
        try await requestVoid("POST", "/auth/logout")
    }

    private struct DeleteAccountRequest: Encodable {
        let password: String
    }

    /// `POST /me/delete` — permanently deletes the calling account
    /// (protocol.md, "Deleting an account"). A POST rather than a
    /// `DELETE /me` because the request carries a body.
    ///
    /// The password is required for the same reason `POST /me/password`
    /// requires it: a live session is not proof, and an unattended
    /// unlocked phone is exactly what this protects against. A wrong one
    /// answers 401 `invalid_credentials`, which the caller must NOT treat
    /// as a dead session — see `ChangePasswordView` for the same trap.
    ///
    /// Succeeding takes every session of the account with it, this one
    /// included, so no further call on this token can succeed: the caller
    /// wipes local state directly rather than going through logout.
    func deleteAccount(password: String) async throws {
        try await requestVoid("POST", "/me/delete", body: DeleteAccountRequest(password: password))
    }

    func me() async throws -> MeResponse {
        try await request("GET", "/me")
    }

    // MARK: - Profile picture

    /// `PUT /me/avatar` — the one endpoint that carries bytes rather than
    /// JSON. Returns the user with the incremented `avatar_version`.
    func uploadAvatar(jpeg: Data) async throws -> UserDTO {
        let (data, _) = try await perform(
            "PUT", "/me/avatar", query: [], bodyData: jpeg, contentType: "image/jpeg")
        let response: AvatarResponse = try decodeResponse(data)
        return response.user
    }

    func deleteAvatar() async throws {
        try await requestVoid("DELETE", "/me/avatar")
    }

    /// Raw bytes of a user's picture, or nil when they have none / are not
    /// visible to us — the server answers both with the same 404.
    func avatar(userID: Int64, version: Int64) async throws -> Data? {
        do {
            // `v` is ignored by the server; it is here so each version is
            // a distinct URL for anything caching along the way.
            let (data, _) = try await perform(
                "GET", "/users/\(userID)/avatar",
                query: [URLQueryItem(name: "v", value: String(version))],
                bodyData: nil)
            return data
        } catch APIError.notFound(_) {
            return nil
        }
    }

    // MARK: - Families

    private struct CreateFamilyRequest: Encodable { let name: String }

    func createFamily(name: String) async throws -> FamilyDTO {
        let response: FamilyResponse = try await request("POST", "/families", body: CreateFamilyRequest(name: name))
        return response.family
    }

    private struct JoinFamilyRequest: Encodable {
        let inviteCode: String
        enum CodingKeys: String, CodingKey { case inviteCode = "invite_code" }
    }

    func joinFamily(inviteCode: String) async throws -> JoinResponse {
        try await request("POST", "/families/join", body: JoinFamilyRequest(inviteCode: inviteCode))
    }

    func myFamily() async throws -> FamilyMineResponse {
        try await request("GET", "/families/mine")
    }

    func rotateInviteCode() async throws -> String {
        let response: InviteCodeResponse = try await request("POST", "/families/invite-code/rotate")
        return response.inviteCode
    }

    /// The body of `PATCH /families/mine`. Which fields are PRESENT decides
    /// what changes, so every one is optional and omitted when untouched.
    ///
    /// `language` is a DOUBLE optional, and that is not a typo. The
    /// protocol gives it three states, not two: an absent key leaves the
    /// family's language alone, an explicit `null` CLEARS it, and a tag
    /// sets it (protocol.md, "The family's language" — the one place in
    /// this protocol where a null means something a missing key does not).
    /// A plain `String?` can only spell two of the three, and
    /// `encodeIfPresent` would silently turn "clear it" into "leave it".
    /// `ai_history` is deliberately NOT such a field: it is a boolean with
    /// a real default and nothing for a null to mean.
    private struct FamilyPatchRequest: Encodable {
        var joinPolicy: String?
        var language: String??
        var aiHistory: Bool?

        enum CodingKeys: String, CodingKey {
            case joinPolicy = "join_policy"
            case language
            case aiHistory = "ai_history"
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encodeIfPresent(joinPolicy, forKey: .joinPolicy)
            if let language {
                // The outer optional is "was this field touched"; the inner
                // is the value. `encodeNil` is what puts a real JSON null
                // on the wire — encodeIfPresent would drop the key.
                if let tag = language {
                    try container.encode(tag, forKey: .language)
                } else {
                    try container.encodeNil(forKey: .language)
                }
            }
            try container.encodeIfPresent(aiHistory, forKey: .aiHistory)
        }
    }

    func setJoinPolicy(_ policy: String) async throws -> FamilyDTO {
        let response: FamilyResponse = try await request(
            "PATCH", "/families/mine", body: FamilyPatchRequest(joinPolicy: policy))
        return response.family
    }

    /// Owner-only: the language `@ai` answers in when it is asked in the
    /// family chat. `nil` CLEARS it back to unset, which is not the same
    /// as choosing English — see FamilyPatchRequest.
    func setFamilyLanguage(_ tag: String?) async throws -> FamilyDTO {
        let response: FamilyResponse = try await request(
            "PATCH", "/families/mine", body: FamilyPatchRequest(language: .some(tag)))
        return response.family
    }

    /// Owner-only: whether a mention of `@ai` in the family chat is shown
    /// the last month of that chat, or only the message that mentioned it.
    func setAIHistory(_ enabled: Bool) async throws -> FamilyDTO {
        let response: FamilyResponse = try await request(
            "PATCH", "/families/mine", body: FamilyPatchRequest(aiHistory: enabled))
        return response.family
    }

    func joinRequests() async throws -> [JoinRequestDTO] {
        let response: JoinRequestsResponse = try await request("GET", "/families/join-requests")
        return response.requests
    }

    func approveJoinRequest(id: Int64) async throws -> MemberDTO {
        let response: MemberResponse = try await request("POST", "/families/join-requests/\(id)/approve")
        return response.member
    }

    func rejectJoinRequest(id: Int64) async throws {
        try await requestVoid("POST", "/families/join-requests/\(id)/reject")
    }

    func leaveFamily() async throws {
        try await requestVoid("POST", "/families/leave")
    }

    func removeMember(userID: Int64) async throws {
        try await requestVoid("DELETE", "/families/members/\(userID)")
    }

    /// Change my own password. The current one is required — a live
    /// session is not proof of knowing it (protocol.md, "Auth").
    ///
    /// Succeeding revokes my OTHER sessions server-side; this one survives,
    /// so there is nothing for the caller to do about its own token.
    func changePassword(current: String, new: String) async throws {
        try await requestVoid(
            "POST", "/me/password",
            body: ChangePasswordRequest(currentPassword: current, newPassword: new))
    }

    /// Owner-only: set a member's password without knowing their old one.
    /// Every session that member has is revoked, so their devices return
    /// to login — which is what makes this a recovery.
    func resetMemberPassword(userID: Int64, newPassword: String) async throws {
        try await requestVoid(
            "POST", "/families/members/\(userID)/password",
            body: ResetPasswordRequest(newPassword: newPassword))
    }

    private struct ChangePasswordRequest: Encodable {
        let currentPassword: String
        let newPassword: String

        enum CodingKeys: String, CodingKey {
            case currentPassword = "current_password"
            case newPassword = "new_password"
        }
    }

    private struct ResetPasswordRequest: Encodable {
        let newPassword: String

        enum CodingKeys: String, CodingKey {
            case newPassword = "new_password"
        }
    }

    // MARK: - Birthdays

    // Two endpoints, and whose birthday it is decides which. The shape is
    // the avatar's on purpose (protocol.md, "Birthdays"): a PUT that
    // replaces whatever was there and a DELETE that clears it, because it
    // is the same kind of thing — a small optional piece of a profile.

    /// My own. Anyone, for themselves.
    func setMyBirthday(month: Int, day: Int) async throws -> UserDTO {
        let response: UserResponse = try await request(
            "PUT", "/me/birthday", body: BirthdayRequest(month: month, day: day))
        return response.user
    }

    func clearMyBirthday() async throws {
        try await requestVoid("DELETE", "/me/birthday")
    }

    /// Somebody else's, as the owner — a parent for a child, typically,
    /// since the child is never going to open a settings screen to type it.
    /// The owner MAY name themselves here, unlike the password reset:
    /// there is no proof being skipped.
    func setMemberBirthday(userID: Int64, month: Int, day: Int) async throws -> MemberDTO {
        let response: MemberResponse = try await request(
            "PUT", "/families/members/\(userID)/birthday",
            body: BirthdayRequest(month: month, day: day))
        return response.member
    }

    func clearMemberBirthday(userID: Int64) async throws {
        try await requestVoid("DELETE", "/families/members/\(userID)/birthday")
    }

    private struct BirthdayRequest: Encodable {
        let month: Int
        let day: Int
    }

    // MARK: - Chats & messages

    func chats() async throws -> ChatsResponse {
        try await request("GET", "/chats")
    }

    private struct DirectChatRequest: Encodable {
        let userID: Int64
        enum CodingKeys: String, CodingKey { case userID = "user_id" }
    }

    func createDirectChat(userID: Int64) async throws -> ChatDTO {
        let response: ChatResponse = try await request("POST", "/chats/direct", body: DirectChatRequest(userID: userID))
        return response.chat
    }

    /// History/catch-up pages. `beforeID` XOR `afterID` per protocol.md;
    /// passing both is a caller bug the server would reject.
    func messages(chatID: Int64, beforeID: Int64? = nil, afterID: Int64? = nil, limit: Int = 50) async throws -> [MessageDTO] {
        var query: [URLQueryItem] = [URLQueryItem(name: "limit", value: String(limit))]
        if let beforeID { query.append(URLQueryItem(name: "before_id", value: String(beforeID))) }
        if let afterID { query.append(URLQueryItem(name: "after_id", value: String(afterID))) }
        let response: MessagesResponse = try await request("GET", "/chats/\(chatID)/messages", query: query)
        return response.messages
    }

    private struct SendMessageRequest: Encodable {
        let clientMsgID: String
        let body: String
        /// Absent, never null, on an ordinary message — Encodable omits a
        /// nil optional, which is exactly what the protocol writes.
        let replyToMessageID: Int64?
        let attachmentID: Int64?
        /// What makes the message a poll; the body is then the QUESTION.
        /// Mutually exclusive with `attachmentID` server-side.
        let poll: NewPollRequest?
        enum CodingKeys: String, CodingKey {
            case clientMsgID = "client_msg_id"
            case body
            case replyToMessageID = "reply_to_message_id"
            case attachmentID = "attachment_id"
            case poll
        }
    }

    /// The only thing a client may say about a poll at creation: its
    /// options. Ids, votes and the sequence are all the server's, and the
    /// question is the message body (docs/protocol.md, "Polls").
    ///
    /// Not `nonisolated`/shared with the socket by accident: `ClientFrame`
    /// spells the same object out itself, because a frame is encoded
    /// through a hand-written `encode(to:)` rather than a request struct.
    struct NewPollRequest: Encodable {
        let options: [String]
    }

    /// Idempotent by `clientMsgID`: the server answers a retry with the
    /// original message (200 instead of 201) — never a duplicate.
    func sendMessage(
        chatID: Int64,
        clientMsgID: String,
        body: String,
        replyToMessageID: Int64? = nil,
        attachmentID: Int64? = nil,
        pollOptions: [String]? = nil
    ) async throws -> MessageDTO {
        let response: MessageResponse = try await request(
            "POST", "/chats/\(chatID)/messages",
            body: SendMessageRequest(
                clientMsgID: clientMsgID,
                body: body,
                replyToMessageID: replyToMessageID,
                attachmentID: attachmentID,
                poll: pollOptions.map { NewPollRequest(options: $0) }))
        return response.message
    }

    // MARK: - Photos, videos and files

    /// Upload a prepared file, STREAMING IT FROM DISK.
    ///
    /// `upload(for:fromFile:)`, not a Data body: a 100 MB video read into
    /// memory is 100 MB resident on a phone that also has to render a chat.
    /// Metadata rides in the query string, so there is no multipart body
    /// for either side to assemble or parse.
    /// The one upload with nothing to upload.
    ///
    /// A location is three numbers, and they go in the query string like
    /// every other piece of attachment metadata — there is no body, no
    /// file on disk to stream and delete, and nothing to fetch back
    /// afterwards (docs/protocol.md, "Locations"). It is a separate call
    /// rather than an argument to the one below because that one's whole
    /// subject is a file: it takes a `fileURL`, streams it, and every
    /// caller deletes it afterwards.
    func uploadLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        name: String?
    ) async throws -> AttachmentDTO {
        guard let serverURL else { throw APIError.notConfigured }
        var query = [
            URLQueryItem(name: "kind", value: AttachmentDTO.Kind.location),
            // Plain decimal degrees, and never a localized formatter: a
            // device set to a comma decimal separator would send "55,7558"
            // and the server would refuse a perfectly good coordinate.
            URLQueryItem(name: "latitude", value: Self.coordinateString(latitude)),
            URLQueryItem(name: "longitude", value: Self.coordinateString(longitude)),
        ]
        if let accuracyM { query.append(URLQueryItem(name: "accuracy_m", value: String(accuracyM))) }
        if let name, !name.isEmpty { query.append(URLQueryItem(name: "name", value: name)) }
        guard let url = Self.endpointURL(base: serverURL, path: "/attachments", query: query) else {
            throw APIError.notConfigured
        }
        var request = URLRequest(url: url, timeoutInterval: uploadTimeout)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let (data, http) = try await send(request)
        guard (200..<300).contains(http.statusCode) else {
            throw Self.mapError(status: http.statusCode, data: data)
        }
        let decoded: AttachmentResponse = try decodeResponse(data)
        return decoded.attachment
    }

    /// A coordinate as the wire wants it: decimal degrees, a full stop for
    /// the point, no grouping, and enough places to be exact to a
    /// centimetre — well past what any phone can actually measure.
    static func coordinateString(_ value: Double) -> String {
        String(format: "%.7f", locale: Locale(identifier: "en_US_POSIX"), value)
    }

    func uploadAttachment(
        fileURL: URL,
        mime: String,
        kind: String,
        width: Int?,
        height: Int?,
        durationMS: Int?,
        name: String? = nil
    ) async throws -> AttachmentDTO {
        guard let serverURL else { throw APIError.notConfigured }
        var query = [URLQueryItem(name: "kind", value: kind)]
        if let width { query.append(URLQueryItem(name: "width", value: String(width))) }
        if let height { query.append(URLQueryItem(name: "height", value: String(height))) }
        if let durationMS {
            query.append(URLQueryItem(name: "duration_ms", value: String(durationMS)))
        }
        // Required for a file, ignored otherwise. URLComponents percent-
        // encodes it, so a name with spaces or umlauts survives the trip.
        if let name { query.append(URLQueryItem(name: "name", value: name)) }
        guard let url = Self.endpointURL(base: serverURL, path: "/attachments", query: query) else {
            throw APIError.notConfigured
        }

        var request = URLRequest(url: url, timeoutInterval: uploadTimeout)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(mime, forHTTPHeaderField: "Content-Type")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }

        let (data, response) = try await uploadFromFile(request, fileURL: fileURL)
        guard (200..<300).contains(response.statusCode) else {
            throw Self.mapError(status: response.statusCode, data: data)
        }
        let decoded: AttachmentResponse = try decodeResponse(data)
        return decoded.attachment
    }

    /// The bubble's preview: a small JPEG, so a plain Data body is fine.
    func uploadPreview(attachmentID: Int64, jpeg: Data) async throws {
        _ = try await perform(
            "PUT", "/attachments/\(attachmentID)/preview",
            query: [], bodyData: jpeg, contentType: "image/jpeg")
    }

    /// The bytes of an attachment, or its preview. Returns nil on a 404 —
    /// which is also what "not visible to you" answers.
    func attachmentData(id: Int64, preview: Bool) async throws -> Data? {
        do {
            let path = preview ? "/attachments/\(id)/preview" : "/attachments/\(id)"
            let (data, _) = try await perform("GET", path, query: [], bodyData: nil)
            return data
        } catch APIError.notFound(_) {
            return nil
        }
    }

    /// A URL an AVPlayer can stream from directly, with the session token
    /// attached — the player fetches bytes itself rather than waiting for
    /// the whole video to download.
    func attachmentStreamURL(id: Int64) -> (url: URL, headers: [String: String])? {
        guard let serverURL,
              let url = Self.endpointURL(base: serverURL, path: "/attachments/\(id)")
        else { return nil }
        var headers: [String: String] = [:]
        if let token { headers["Authorization"] = "Bearer \(token)" }
        return (url, headers)
    }

    private func uploadFromFile(
        _ request: URLRequest,
        fileURL: URL
    ) async throws -> (Data, HTTPURLResponse) {
        do {
            let (data, response) = try await session.upload(for: request, fromFile: fileURL)
            guard let http = response as? HTTPURLResponse else {
                throw APIError.transport(URLError(.badServerResponse))
            }
            return (data, http)
        } catch let error as URLError {
            throw APIError.transport(error)
        }
    }

    // MARK: - Board

    private struct CreateNoteRequest: Encodable {
        let text: String
        let color: String
        let x: Double
        let y: Double
    }

    /// Every field optional: a MOVE sends only x/y (any member may), an
    /// edit sends text and/or color (author only). Which fields are present
    /// is what decides the permission the server applies.
    private struct PatchNoteRequest: Encodable {
        let text: String?
        let color: String?
        let x: Double?
        let y: Double?
    }

    func board() async throws -> BoardResponse {
        try await request("GET", "/families/mine/board")
    }

    /// What the family has sent. Visible to every member, not just the
    /// owner (docs/protocol.md, "Family statistics").
    func familyStats() async throws -> FamilyStatsDTO {
        try await request("GET", "/families/mine/stats")
    }

    func boardChanges(afterSeq: Int64, limit: Int) async throws -> [NoteDTO] {
        let response: BoardChangesResponse = try await request(
            "GET", "/families/mine/board/changes",
            query: [
                URLQueryItem(name: "after_seq", value: String(afterSeq)),
                URLQueryItem(name: "limit", value: String(limit)),
            ])
        return response.notes
    }

    func createNote(text: String, color: String, x: Double, y: Double) async throws -> NoteDTO {
        let response: NoteResponse = try await request(
            "POST", "/families/mine/board/notes",
            body: CreateNoteRequest(text: text, color: color, x: x, y: y))
        return response.note
    }

    func patchNote(
        id: Int64,
        text: String? = nil,
        color: String? = nil,
        x: Double? = nil,
        y: Double? = nil
    ) async throws -> NoteDTO {
        let response: NoteResponse = try await request(
            "PATCH", "/families/mine/board/notes/\(id)",
            body: PatchNoteRequest(text: text, color: color, x: x, y: y))
        return response.note
    }

    func deleteNote(id: Int64) async throws {
        try await requestVoid("DELETE", "/families/mine/board/notes/\(id)")
    }

    private struct EditMessageRequest: Encodable {
        let body: String
    }

    /// PATCH the body. Author only; the server answers with the whole
    /// message, edit stamps included.
    func editMessage(chatID: Int64, messageID: Int64, body: String) async throws -> MessageDTO {
        let response: MessageResponse = try await request(
            "PATCH", "/chats/\(chatID)/messages/\(messageID)",
            body: EditMessageRequest(body: body))
        return response.message
    }

    /// One page of the edit catch-up: whole messages, ordered by edit_seq.
    func edits(chatID: Int64, afterSeq: Int64, limit: Int) async throws -> [MessageDTO] {
        let response: MessagesResponse = try await request(
            "GET", "/chats/\(chatID)/edits",
            query: [
                URLQueryItem(name: "after_seq", value: String(afterSeq)),
                URLQueryItem(name: "limit", value: String(limit)),
            ])
        return response.messages
    }

    private struct ReadRequest: Encodable {
        let lastReadMessageID: Int64
        enum CodingKeys: String, CodingKey { case lastReadMessageID = "last_read_message_id" }
    }

    func markRead(chatID: Int64, lastReadMessageID: Int64) async throws {
        try await requestVoid("POST", "/chats/\(chatID)/read", body: ReadRequest(lastReadMessageID: lastReadMessageID))
    }

    // MARK: - Reactions

    private struct SetReactionRequest: Encodable {
        let emoji: String
    }

    /// Set or replace the caller's reaction — an idempotent state-set,
    /// not a toggle (the coordinator decides set vs remove locally).
    /// Returns the message's full authoritative reaction state.
    func setReaction(chatID: Int64, messageID: Int64, emoji: String) async throws -> ReactionStateDTO {
        try await request("PUT", "/chats/\(chatID)/messages/\(messageID)/reaction",
                          body: SetReactionRequest(emoji: emoji))
    }

    /// Remove the caller's reaction; idempotent (removing nothing returns
    /// the current state unchanged, seq included).
    func removeReaction(chatID: Int64, messageID: Int64) async throws -> ReactionStateDTO {
        try await request("DELETE", "/chats/\(chatID)/messages/\(messageID)/reaction")
    }

    /// Reaction catch-up pages, ascending by reaction_seq; the caller
    /// loops (advancing afterSeq) until a short page, like `after_id`.
    func reactions(chatID: Int64, afterSeq: Int64, limit: Int = 50) async throws -> [ReactionStateDTO] {
        let query = [
            URLQueryItem(name: "after_seq", value: String(afterSeq)),
            URLQueryItem(name: "limit", value: String(limit)),
        ]
        let response: MessageReactionsResponse = try await request("GET", "/chats/\(chatID)/reactions", query: query)
        return response.messageReactions
    }

    // MARK: - Polls

    private struct VoteRequest: Encodable {
        let optionID: Int64
        enum CodingKeys: String, CodingKey { case optionID = "option_id" }
    }

    /// Set the caller's choice — an idempotent state-set, not a toggle
    /// (the coordinator decides set vs retract locally, exactly as it does
    /// for a reaction). Returns the poll's full authoritative state.
    func vote(chatID: Int64, messageID: Int64, optionID: Int64) async throws -> PollStateDTO {
        try await request("PUT", "/chats/\(chatID)/messages/\(messageID)/vote",
                          body: VoteRequest(optionID: optionID))
    }

    /// Retract the caller's vote; idempotent (retracting nothing returns
    /// the current state unchanged and burns no sequence value).
    func retractVote(chatID: Int64, messageID: Int64) async throws -> PollStateDTO {
        try await request("DELETE", "/chats/\(chatID)/messages/\(messageID)/vote")
    }

    /// Close a poll. Author only and ONE-WAY; closing a closed poll is a
    /// no-op. A non-author gets 403 `not_message_author`.
    func closePoll(chatID: Int64, messageID: Int64) async throws -> PollStateDTO {
        try await request("POST", "/chats/\(chatID)/messages/\(messageID)/poll/close")
    }

    /// Poll catch-up pages, ascending by poll_seq; the caller loops
    /// (advancing afterSeq) until a short page, like `after_id`.
    func polls(chatID: Int64, afterSeq: Int64, limit: Int = 50) async throws -> [PollStateDTO] {
        let query = [
            URLQueryItem(name: "after_seq", value: String(afterSeq)),
            URLQueryItem(name: "limit", value: String(limit)),
        ]
        let response: ChatPollsResponse = try await request("GET", "/chats/\(chatID)/polls", query: query)
        return response.polls
    }

    // MARK: - Devices

    private struct DeviceRequest: Encodable {
        let platform: String
        let pushToken: String?
        enum CodingKeys: String, CodingKey {
            case platform
            case pushToken = "push_token"
        }
        // push_token must be an explicit JSON null, not an absent key.
        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(platform, forKey: .platform)
            try container.encode(pushToken, forKey: .pushToken)
        }
    }

    func registerDevice(platform: String, pushToken: String?) async throws -> Int64 {
        let response: DeviceResponse = try await request("POST", "/devices", body: DeviceRequest(platform: platform, pushToken: pushToken))
        return response.deviceID
    }

    func deleteDevice(id: Int64) async throws {
        try await requestVoid("DELETE", "/devices/\(id)")
    }

    // MARK: - Request plumbing

    /// `{base}/api/v1{path}` — path segments are appended verbatim, so
    /// callers pass "/chats/42/messages" style paths.
    private static func endpointURL(base: URL, path: String, query: [URLQueryItem] = []) -> URL? {
        var text = base.absoluteString
        if text.hasSuffix("/") { text.removeLast() }
        guard var components = URLComponents(string: text + "/api/v1" + path) else { return nil }
        if !query.isEmpty {
            components.queryItems = query
            // URLComponents leaves '+' literal in a query value, and the
            // other end reads a query string as form-urlencoded, where '+'
            // MEANS a space. So "C++ notes.pdf" arrives as "C   notes.pdf"
            // — silently, and only for the one character nobody tests.
            components.percentEncodedQuery = components.percentEncodedQuery?
                .replacingOccurrences(of: "+", with: "%2B")
        }
        return components.url
    }

    private func request<Response: Decodable>(
        _ method: String,
        _ path: String,
        query: [URLQueryItem] = []
    ) async throws -> Response {
        let (data, _) = try await perform(method, path, query: query, bodyData: nil)
        return try decodeResponse(data)
    }

    private func request<Response: Decodable>(
        _ method: String,
        _ path: String,
        query: [URLQueryItem] = [],
        body: some Encodable
    ) async throws -> Response {
        let bodyData = try? APICoding.encoder().encode(body)
        let (data, _) = try await perform(method, path, query: query, bodyData: bodyData)
        return try decodeResponse(data)
    }

    private func requestVoid(_ method: String, _ path: String) async throws {
        _ = try await perform(method, path, query: [], bodyData: nil)
    }

    private func requestVoid(_ method: String, _ path: String, body: some Encodable) async throws {
        let bodyData = try? APICoding.encoder().encode(body)
        _ = try await perform(method, path, query: [], bodyData: bodyData)
    }

    private func decodeResponse<Response: Decodable>(_ data: Data) throws -> Response {
        do {
            return try APICoding.decoder().decode(Response.self, from: data)
        } catch {
            AppLog.api.error("Response decode failed: \(String(describing: error))")
            throw APIError.decoding
        }
    }

    /// Issue one request; on a GET that hits a transient status (429/5xx),
    /// wait per Retry-After (capped) and retry exactly once. Returns the
    /// (data, response) of a 2xx, or throws the mapped APIError.
    private func perform(
        _ method: String,
        _ path: String,
        query: [URLQueryItem],
        bodyData: Data?,
        contentType: String = "application/json"
    ) async throws -> (Data, HTTPURLResponse) {
        guard let serverURL else { throw APIError.notConfigured }
        guard let url = Self.endpointURL(base: serverURL, path: path, query: query) else {
            throw APIError.notConfigured
        }

        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let bodyData {
            request.setValue(contentType, forHTTPHeaderField: "Content-Type")
            request.httpBody = bodyData
        }

        var (data, http) = try await send(request)

        if method == "GET", Self.isTransient(http.statusCode) {
            let wait = Self.retryDelay(from: http, fallback: 0.5)
            AppLog.api.info("GET \(path, privacy: .public) returned \(http.statusCode, privacy: .public); retrying once in \(wait, privacy: .public)s")
            try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
            (data, http) = try await send(request)
        }

        guard (200..<300).contains(http.statusCode) else {
            throw Self.mapError(status: http.statusCode, data: data)
        }
        return (data, http)
    }

    private func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        do {
            let (data, response) = try await session.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw APIError.transport(URLError(.badServerResponse))
            }
            return (data, http)
        } catch let error as URLError {
            throw APIError.transport(error)
        }
    }

    /// 429 (throttled) and any 5xx are worth the single GET retry.
    private static func isTransient(_ status: Int) -> Bool {
        status == 429 || (500..<600).contains(status)
    }

    /// Seconds before the retry: `Retry-After` (delta-seconds form) when
    /// the server sent one, otherwise `fallback`. Capped at 5 s — beyond
    /// that the UI's own error handling is a better experience than a hang.
    private static func retryDelay(from http: HTTPURLResponse, fallback: TimeInterval) -> TimeInterval {
        if let header = http.value(forHTTPHeaderField: "Retry-After"),
           let seconds = TimeInterval(header.trimmingCharacters(in: .whitespaces)),
           seconds >= 0 {
            return min(seconds, 5)
        }
        return fallback
    }

    /// Map a non-2xx to a typed error, extracting the documented
    /// `{"error":{code,message}}` body when present. A body that fails to
    /// parse degrades to nil code/message, never to a decode error — the
    /// status alone still routes correctly.
    private static func mapError(status: Int, data: Data) -> APIError {
        let body = try? APICoding.decoder().decode(APIErrorBody.self, from: data)
        let code = body?.error.code
        let message = body?.error.message
        switch status {
        case 401: return .unauthorized
        case 403: return .forbidden(code: code)
        case 404: return .notFound(code: code)
        case 413: return .payloadTooLarge
        case 400..<500: return .conflict(code: code, message: message)
        default: return .server(status: status, message: message)
        }
    }
}
