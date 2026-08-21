#!/bin/bash
#
# Seed the fixture for ios/FamilyConnectUITests/ConversationScrollUITests:
# a fresh local server on 127.0.0.1:8091 (PostgreSQL in the fc-test-pg
# docker container) with users olive/junior, one family, 1500 messages of
# varied length backdated across ~7 weeks, and a few reactions. Re-run
# before every test invocation — the test itself posts extra data.
#
#   server/scripts/seed-scroll-uitest.sh
#   cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
#     TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 xcodebuild test … \
#     -only-testing:FamilyConnectUITests/ConversationScrollUITests
#
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
pkill -f "family-connect" 2>/dev/null || true; sleep 1
docker exec fc-test-pg psql -U postgres -c "DROP DATABASE IF EXISTS family_connect_dev WITH (FORCE);" -c "CREATE DATABASE family_connect_dev;" >/dev/null
cd "$SCRIPT_DIR/.."
(cargo run --quiet -- --config "$SCRIPT_DIR/uitest-server.toml" > /tmp/fc-uitest-server.log 2>&1 &)
sleep 5
curl -s http://127.0.0.1:8091/api/v1/healthz
COUNT="${1:-1500}" python3 - << 'EOF'
import json, urllib.request, uuid
BASE = "http://127.0.0.1:8091/api/v1"
def call(method, path, token=None, body=None):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("Content-Type", "application/json")
    if token: req.add_header("Authorization", f"Bearer {token}")
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(req, data) as r:
        raw = r.read()
        return json.loads(raw) if raw else {}
ot = call("POST", "/auth/register", body={"username":"olive","display_name":"Olive","password":"password123"})["token"]
jt = call("POST", "/auth/register", body={"username":"junior","display_name":"Junior","password":"password123"})["token"]
fam = call("POST", "/families", ot, {"name":"The Smiths"})
call("PATCH", "/families/mine", ot, {"join_policy":"open"})
call("POST", "/families/join", jt, {"invite_code": fam["family"]["invite_code"]})
chat_id = next(c["chat"]["id"] for c in call("GET", "/chats", ot)["chats"] if c["chat"]["kind"] == "family")
long_tail = " The plan for the weekend involves the market in the morning, then lunch at the lake if the weather holds, and we should not forget to pick up the package before six."
huge_tail = " " + "This paragraph repeats the way real families paste plans, shopping lists and long letters into the chat, which makes a single bubble taller than the whole screen. " * 8
mega_tail = " " + "A truly enormous pasted note near the protocol limit so one bubble is many screens tall and every lazy height estimate around it is wrong by an order of magnitude. " * 7
import os
count = int(os.environ.get("COUNT", "1500"))
ids = []
for i in range(1, count + 1):
    if i % 41 == 0:
        body = f"Message number {i}" + huge_tail
    elif i % 16 == 7:
        body = f"Message number {i}" + mega_tail
    elif i % 7 == 0:
        body = f"Message number {i}" + long_tail
    else:
        body = f"Message number {i}" + ("" if i % 3 else " with a somewhat longer body that wraps over two lines on a phone")
    ids.append(call("POST", f"/chats/{chat_id}/messages", ot if i % 2 else jt, {"client_msg_id": str(uuid.uuid4()), "body": body})["message"]["id"])
for idx, emoji, tok in [(2,"❤️",jt),(count//3,"😂",ot),(count-5,"👍",jt),(count-2,"😮",jt),(count-2,"❤️",ot)]:
    call("PUT", f"/chats/{chat_id}/messages/{ids[idx]}/reaction", tok, {"emoji": emoji})
call("POST", f"/chats/{chat_id}/read", jt, {"last_read_message_id": ids[-1]})
print(f" seeded chat {chat_id}: {count} messages")
EOF
COUNT="${1:-1500}"
docker exec fc-test-pg psql -U postgres -d family_connect_dev -q -c "UPDATE messages SET created_at = now() - (($COUNT - id) * interval '47 minutes');"
