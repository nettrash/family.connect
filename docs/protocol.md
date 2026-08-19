# Family Connect protocol — v1

This document is the single source of truth for the API between the Family Connect server
(`server/`) and the iOS (`ios/`) and Android (`android/`) clients. When code and this document
disagree, this document wins; fix the code.

## Transport

- REST: JSON over HTTP under `{base}/api/v1`. `{base}` is the user-entered server URL
  (e.g. `https://chat.example.com` or `http://192.168.1.10:8080`).
- Realtime: WebSocket at `{base}/api/v1/ws` (scheme `wss://` for https servers, `ws://` for http).
- In production the server binds to loopback and sits behind nginx, which terminates TLS and
  proxies both REST and the WebSocket upgrade.

## Compatibility rules

- Clients MUST ignore unknown JSON fields in any response, and unknown `type` values in any
  WebSocket frame. The server likewise ignores unknown client frame types (logged at debug).
  This is how voice/video signaling frames (`call_offer`, `call_answer`, …) will be added later
  without breaking v1 clients.
- All field names are `snake_case`. Timestamps are RFC 3339 UTC strings
  (e.g. `"2026-08-19T17:03:12Z"`). All ids are 64-bit integers.

## Authentication

- Opaque per-device session tokens. `POST /auth/register` and `POST /auth/login` each create a
  new session and return its token exactly once. The server stores only a SHA-256 hash.
- Every other endpoint (and the WebSocket upgrade) requires `Authorization: Bearer <token>`.
- Sessions have a sliding expiry (default 180 days, refreshed by use). A `401` means the session
  is gone — the client wipes local state (keeping the server URL) and returns to login.

## Error shape

Non-2xx responses carry:

```json
{"error": {"code": "username_taken", "message": "username is already in use"}}
```

`code` is a stable machine-readable string; `message` is a human-readable English sentence.
Canonical codes: `unauthorized`, `invalid_credentials`, `username_taken`, `validation`,
`already_in_family`, `not_in_family`, `not_family_owner`, `invalid_invite_code`,
`join_request_pending`, `join_request_not_pending`, `user_already_in_family`,
`owner_cannot_leave`, `cannot_remove_owner`, `cannot_dm_self`, `not_same_family`,
`user_not_found`, `chat_not_found`, `not_chat_member`, `message_empty`, `message_too_long`,
`invalid_pagination`, `device_not_found`, `internal`.

## Objects

```json
User      {"id": 7, "username": "anna", "display_name": "Anna", "created_at": "…"}
Member    {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner|member"}
Family    {"id": 3, "name": "The Smiths", "join_policy": "open|approval", "created_at": "…"}
          — plus "invite_code": "ABCD2345" when (and only when) the caller is the owner
JoinRequest {"id": 12, "user": {User}, "created_at": "…"}
Chat      {"id": 42, "kind": "family|direct", "title": "The Smiths", "peer_user_id": 9|null}
          — "title" is the family name for the family chat, the peer's display name for direct;
            "peer_user_id" is set for direct chats only
Message   {"id": 1338, "chat_id": 42, "sender_id": 7,
           "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
           "body": "Dinner at 7?", "created_at": "…"}
```

## REST endpoints

### Auth

| Method & path | Body → Response |
|---|---|
| `POST /auth/register` | `{username, display_name, password}` → `201 {token, user: User}`. Username: 3–32 chars `[a-zA-Z0-9_.]`, case-insensitively unique. Password ≥ 8 chars. Display name 1–64 chars. Errors: `username_taken`, `validation`. |
| `POST /auth/login` | `{username, password}` → `200 {token, user: User}`. Error: `invalid_credentials` (401). |
| `POST /auth/logout` | (auth) → `204`. Revokes the calling session and closes its sockets. |
| `GET /me` | (auth) → `200 {user: User, family: Family\|null, role: "owner"\|"member"\|null, pending_join_request: {family_id, family_name, created_at}\|null}`. `pending_join_request` is the caller's live join request, if any — a client that was waiting and sees neither `family` nor `pending_join_request` knows the request was rejected. |

### Families

| Method & path | Body → Response |
|---|---|
| `POST /families` | `{name}` (1–64 chars) → `201 {family: Family}`. Caller becomes owner; the family chat is created automatically. Error: `already_in_family`. |
| `POST /families/join` | `{invite_code}` → `200 {status: "joined"}` (policy `open` — membership immediate) or `200 {status: "pending"}` (policy `approval` — join request created). Errors: `invalid_invite_code` (404), `already_in_family`, `join_request_pending`. |
| `GET /families/mine` | → `200 {family: Family, members: [Member]}`. `family.invite_code` present for the owner only. Error: `not_in_family`. |
| `POST /families/invite-code/rotate` | (owner) → `200 {invite_code}`. Old code stops working; pending requests survive. |
| `PATCH /families/mine` | (owner) `{join_policy: "open"\|"approval"}` → `200 {family: Family}`. |
| `GET /families/join-requests` | (owner) → `200 {requests: [JoinRequest]}` (pending only). |
| `POST /families/join-requests/{id}/approve` | (owner) → `200 {member: Member}`. Errors: `join_request_not_pending`, `user_already_in_family`. |
| `POST /families/join-requests/{id}/reject` | (owner) → `204`. Error: `join_request_not_pending`. |
| `POST /families/leave` | → `204`. The owner may leave only as the sole member (the family is then deleted); otherwise `409 owner_cannot_leave`. Leaving removes the caller from the family chat and their direct chats; history is retained and resurfaces on rejoin. |
| `DELETE /families/members/{user_id}` | (owner) → `204`. Error: `cannot_remove_owner`. |

### Chats & messages

| Method & path | Body → Response |
|---|---|
| `GET /chats` | → `200 {chats: [{chat: Chat, last_message: Message\|null, unread_count: 3}]}`. Family chat included always; direct chats once they exist. |
| `POST /chats/direct` | `{user_id}` → `200 {chat: Chat}` — get-or-create, idempotent. Errors: `cannot_dm_self`, `not_same_family`, `user_not_found`. |
| `GET /chats/{id}/messages` | Query: `before_id` XOR `after_id` (optional), `limit` (default 50, max 200) → `200 {messages: [Message]}`. `before_id`: strictly older, **newest-first** (history pages). `after_id`: strictly newer, **oldest-first** (reconnect catch-up). Neither: the newest `limit`, newest-first. Errors: `chat_not_found`, `not_chat_member`, `invalid_pagination`. |
| `POST /chats/{id}/messages` | `{client_msg_id: "<uuid>", body}` → `201 {message: Message}`. Retrying with the same `client_msg_id` returns the existing message as `200` — never a duplicate. Body: trimmed, non-empty, ≤ 4000 chars. Errors: `message_empty`, `message_too_long`, `not_chat_member`. |
| `POST /chats/{id}/read` | `{last_read_message_id}` → `204`. Monotonic — the server keeps the max ever reported. |

### Devices (push hook — no delivery in v1)

| Method & path | Body → Response |
|---|---|
| `POST /devices` | `{platform: "ios"\|"android", push_token: string\|null}` → `201 {device_id}`. Upserts by token when non-null. |
| `DELETE /devices/{id}` | → `204`. Error: `device_not_found`. |

### Ops

| Method & path | Response |
|---|---|
| `GET /healthz` | `200 {"status": "ok"}` (checks the database). No auth. |

## WebSocket protocol

Connect with `Authorization: Bearer <token>` on the upgrade request. A bad token fails the
upgrade with `401`. The server closes with code `4401` when the session expires mid-connection.

Frames are JSON text messages tagged by `"type"`.

### Client → server

```json
{"type": "send",   "chat_id": 42, "client_msg_id": "8f14e45f-…", "body": "Dinner at 7?"}
{"type": "read",   "chat_id": 42, "last_read_message_id": 1337}
{"type": "typing", "chat_id": 42}
{"type": "ping"}
```

### Server → client

```json
{"type": "ack",     "client_msg_id": "8f14e45f-…", "message": {Message}}
{"type": "message", "message": {Message}}
{"type": "read",    "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}
{"type": "typing",  "chat_id": 42, "user_id": 9}
{"type": "member_joined", "family_id": 3, "user": {"id": 11, "username": "junior", "display_name": "Junior"}}
{"type": "member_left",   "family_id": 3, "user_id": 11}
{"type": "pong"}
{"type": "error",   "code": "not_chat_member", "message": "…", "client_msg_id": "8f14e45f-…"}
```

(`client_msg_id` on `error` is present when the error answers a `send`.)

### Semantics

- **Ack fan-out**: the connection that sent a `send` frame receives `ack`; every other
  connection of every chat member — including the sender's other devices — receives `message`.
  A message posted over REST is acked by the HTTP response; all connections (including the
  sender's own) receive `message`.
- **Dedup**: `(chat_id, sender, client_msg_id)` is unique server-side. A retried `send` re-emits
  the `ack` with the original message and does not re-fan-out.
- **Read/typing**: relayed to the other members of the chat. `typing` is never persisted and is
  throttled server-side to one per chat per 3 s per connection.
- **Keepalive**: the server pings (WS protocol level) every 30 s and drops sockets idle for
  75 s. Clients should send `{"type":"ping"}` every ~25 s if their WS library hides protocol
  pings, and treat a missing `pong` as a dead connection.
- **Best-effort delivery**: the socket is a live wire, not a queue. A slow client's socket is
  dropped; REST is the source of truth. On every (re)connect a client must resync:
  1. `GET /me` — reconcile membership.
  2. `GET /chats` — chat list, previews, authoritative unread counts.
  3. Per chat: `GET /chats/{id}/messages?after_id=<max known message id>` looped until a short
     page — message ids are globally monotonic, so `max(id)` is the sync cursor.
  4. Re-send any locally pending outbound messages (safe: `client_msg_id` dedups).

## Limits (server defaults, configurable)

| Limit | Default |
|---|---|
| Message body | 4000 chars |
| Page size | 50 default / 200 max |
| HTTP body | 16 KiB |
| Per-socket outbound queue | 64 frames |
| Session TTL | 180 days, sliding |
