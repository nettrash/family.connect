#!/bin/bash
#
# Seed the fixture for ios/FamilyConnectUITests/AttachmentViewerSwipeUITests:
# a fresh local server on 127.0.0.1:8091 (PostgreSQL in the fc-test-pg
# docker container) with users olive/junior, one family, five short
# messages and ONE album message of three solid-colour photos. Re-run
# before every test invocation.
#
#   server/scripts/seed-album-uitest.sh
#   cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
#     TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 xcodebuild test … \
#     -only-testing:FamilyConnectUITests/AttachmentViewerSwipeUITests
#
# Build the app SIGNED for this (no CODE_SIGNING_ALLOWED=NO): the app-group
# entitlement the share extension needs makes the Keychain refuse an
# unsigned build's token write, and the login screen then says "Something
# went wrong".
#
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
pkill -f "family-connect" 2>/dev/null || true; sleep 1
docker exec fc-test-pg psql -U postgres -c "DROP DATABASE IF EXISTS family_connect_dev WITH (FORCE);" -c "CREATE DATABASE family_connect_dev;" >/dev/null
cd "$SCRIPT_DIR/.."
(cargo run --quiet -- --config "$SCRIPT_DIR/uitest-server.toml" > /tmp/fc-uitest-server.log 2>&1 &)
for _ in $(seq 1 60); do
  curl -sf http://127.0.0.1:8091/api/v1/healthz >/dev/null 2>&1 && break
  sleep 2
done
curl -s http://127.0.0.1:8091/api/v1/healthz; echo
python3 - << 'PY'
import json, urllib.request, uuid, zlib, struct
BASE = "http://127.0.0.1:8091/api/v1"
def call(method, path, token=None, body=None, raw=None, ctype="application/json"):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("Content-Type", ctype)
    if token: req.add_header("Authorization", f"Bearer {token}")
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    with urllib.request.urlopen(req, data) as r:
        out = r.read()
        return json.loads(out) if out else {}
def png(w, h, rgb):
    # A solid-colour PNG from the standard library alone: no PIL needed.
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    row = b"\x00" + bytes(rgb) * w
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(row * h, 9)) + chunk(b"IEND", b""))
ot = call("POST", "/auth/register", body={"username":"olive","display_name":"Olive","password":"password123"})["token"]
jt = call("POST", "/auth/register", body={"username":"junior","display_name":"Junior","password":"password123"})["token"]
fam = call("POST", "/families", ot, {"name":"The Smiths"})
call("PATCH", "/families/mine", ot, {"join_policy":"open"})
call("POST", "/families/join", jt, {"invite_code": fam["family"]["invite_code"]})
chat_id = next(c["chat"]["id"] for c in call("GET", "/chats", ot)["chats"] if c["chat"]["kind"] == "family")
for i in range(1, 6):
    call("POST", f"/chats/{chat_id}/messages", ot if i % 2 else jt, {"client_msg_id": str(uuid.uuid4()), "body": f"Message number {i}"})
ids = []
for rgb in [(200, 40, 40), (40, 160, 60), (40, 80, 220)]:
    a = call("POST", "/attachments?kind=photo&width=800&height=600", ot, raw=png(800, 600, rgb), ctype="image/png")
    ids.append(a["attachment"]["id"])
m = call("POST", f"/chats/{chat_id}/messages", ot, {"client_msg_id": str(uuid.uuid4()), "body": "Album test", "attachment_ids": ids})
print(f" seeded chat {chat_id}: album message {m['message']['id']} with attachments {ids}")
PY
