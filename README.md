# Family Connect

A self-hosted chat for one household — or several. The server runs on your own Ubuntu box,
the iOS and Android apps talk only to it, and no third party ever sees a message. One server
instance can host multiple families; each user belongs to exactly one family, every family has
a single all-members chat, and any two members of the same family can talk one-to-one. Text
messages in v1; the protocol is shaped so voice/video call signaling can be added later without
breaking older clients.

## Repository layout

```
family.connect/
├── docs/
│   └── protocol.md        # REST + WebSocket contract — the single source of truth
├── server/                # Rust server (Axum + PostgreSQL), systemd + nginx artifacts
├── ios/                   # SwiftUI client, iOS 17+ (FamilyConnect.xcodeproj)
├── android/               # Jetpack Compose client, Android 8+ (Gradle, :app)
├── CHANGELOG              # plain-text release history
└── LICENSE                # MIT
```

## How it fits together

- **Server** (`server/`): a single static binary behind nginx. PostgreSQL stores users,
  families, chats and messages; migrations run automatically at startup. Realtime delivery is
  a WebSocket fan-out; REST is the source of truth clients resync from after any disconnect.
  Sessions are opaque bearer tokens (only their SHA-256 hashes are stored). Push notifications
  are a hook in v1: device tokens are collected and delivery events logged, but nothing is sent
  until an APNs/FCM transport is configured in a later version.
- **Clients** (`ios/`, `android/`): Telegram-simple. First run asks for the server address,
  then register or log in, then create a family or join one with an invite code (family owners
  choose whether a code joins immediately or requires their approval). Both apps cache history
  locally, send optimistically with retry, and resync over REST whenever the socket reconnects.

## Building

Each component builds independently; see `docs/protocol.md` for the wire contract.

```bash
# Server (Rust 1.88+)
cd server
cargo build --release
cargo test                                    # unit tests
cargo test -- --ignored                       # integration tests; need PG* env vars pointing
                                              # at a reachable PostgreSQL

# Android
cd android
./gradlew assembleDebug testDebugUnitTest lintDebug

# iOS
cd ios
xcodebuild test -project FamilyConnect.xcodeproj -scheme FamilyConnect \
  -destination 'platform=iOS Simulator,name=iPhone 16' CODE_SIGNING_ALLOWED=NO
```

## Installing the server (Ubuntu)

```bash
# 1. PostgreSQL
sudo apt install postgresql
sudo -u postgres psql -c "CREATE ROLE family_connect LOGIN PASSWORD 'change-me';"
sudo -u postgres psql -c "CREATE DATABASE family_connect OWNER family_connect;"
# (the schema is created automatically the first time the service starts)

# 2. Binary
cd server && cargo build --release
sudo install -m 0755 target/release/family-connect /usr/local/bin/

# 3. Service user
sudo useradd --system --no-create-home --shell /usr/sbin/nologin family-connect

# 4. Config
sudo mkdir -p /etc/family-connect
sudo cp server/config.example.toml /etc/family-connect/config.toml
sudo $EDITOR /etc/family-connect/config.toml        # set the [database] password
sudo chown root:family-connect /etc/family-connect/config.toml
sudo chmod 0640 /etc/family-connect/config.toml

# 5. systemd
sudo cp server/systemd/family-connect.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now family-connect

# 6. nginx + TLS
sudo cp server/nginx/family-connect.conf /etc/nginx/sites-available/family-connect
# edit server_name, then:
sudo ln -s /etc/nginx/sites-available/family-connect /etc/nginx/sites-enabled/
sudo certbot --nginx -d chat.example.com
sudo systemctl reload nginx

# 7. Smoke test
curl https://chat.example.com/api/v1/healthz        # → {"status":"ok"}
journalctl -u family-connect -f                     # logs
```

## Trying it end to end

1. Start the server (locally: `cargo run -- --config ./dev-config.toml` with `[server] bind`
   on `0.0.0.0:8080` and a local database).
2. On two phones / simulators, enter the server address (`http://<your-mac-ip>:8080` works on a
   LAN — both apps allow plain HTTP for local networks and warn about it).
3. Register two users. Have the first create a family and share its invite code; have the
   second join with it. Both land in the family chat; either can start a private chat from the
   member list.

## Security notes

- TLS is nginx's job; the server itself binds to loopback in the shipped config.
- Passwords are argon2id hashes; session tokens are 256-bit random values stored only as
  SHA-256 hashes, with sliding 180-day expiry and instant revocation on logout.
- The systemd unit runs as a dedicated non-login user with a hardened sandbox
  (`ProtectSystem=strict`, `NoNewPrivileges`, restricted address families and syscalls).
- Messages are stored in plaintext in *your* PostgreSQL — the privacy model is "your household,
  your box", not end-to-end encryption.

## License

[MIT](./LICENSE)
