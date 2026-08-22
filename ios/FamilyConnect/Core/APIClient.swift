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

    private struct JoinPolicyRequest: Encodable {
        let joinPolicy: String
        enum CodingKeys: String, CodingKey { case joinPolicy = "join_policy" }
    }

    func setJoinPolicy(_ policy: String) async throws -> FamilyDTO {
        let response: FamilyResponse = try await request("PATCH", "/families/mine", body: JoinPolicyRequest(joinPolicy: policy))
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
        enum CodingKeys: String, CodingKey {
            case clientMsgID = "client_msg_id"
            case body
            case replyToMessageID = "reply_to_message_id"
        }
    }

    /// Idempotent by `clientMsgID`: the server answers a retry with the
    /// original message (200 instead of 201) — never a duplicate.
    func sendMessage(
        chatID: Int64,
        clientMsgID: String,
        body: String,
        replyToMessageID: Int64? = nil
    ) async throws -> MessageDTO {
        let response: MessageResponse = try await request(
            "POST", "/chats/\(chatID)/messages",
            body: SendMessageRequest(
                clientMsgID: clientMsgID,
                body: body,
                replyToMessageID: replyToMessageID))
        return response.message
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
        if !query.isEmpty { components.queryItems = query }
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
