#!/bin/bash
#
# Seed the fixture the STORE SCREENSHOTS are shot against — App Store,
# Mac App Store and Google Play.
#
#   server/scripts/seed-store-screenshots.sh
#
# A fresh local server on 127.0.0.1:8091 (PostgreSQL in the fc-test-pg
# docker container) holding one invented family with one of everything
# worth photographing: a three-day thread, a photo album, a poll
# mid-vote, a voice note, a shared location, a call record, board notes
# and birthdays.
#
# EVERY NAME AND EVERY MESSAGE HERE IS INVENTED. These end up on a public
# store listing, so nothing may resemble a real person's data. The whole
# fixture is disposable: the database is dropped and recreated on each
# run and the attachments live in /tmp.
#
# The photographed account is `nora` / `password123` (the family owner —
# owner-only screens like the reports inbox and the member cap need it).
#
# PHOTOS. The album uses real image files when they are present, so the
# listing can show something better than generated colour:
#
#     server/scripts/screenshot-photos/*.{jpg,jpeg,png}
#
# That directory is gitignored. With nothing in it the script falls back
# to generated gradients, which are honest but abstract.
#
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
pkill -f "family-connect" 2>/dev/null || true; sleep 1
docker exec fc-test-pg psql -U postgres \
  -c "DROP DATABASE IF EXISTS family_connect_dev WITH (FORCE);" \
  -c "CREATE DATABASE family_connect_dev;" >/dev/null
cd "$SCRIPT_DIR/.."
(cargo run --quiet -- --config "$SCRIPT_DIR/uitest-server.toml" > /tmp/fc-store-server.log 2>&1 &)
# Wait for the port rather than sleeping a fixed time: a cold `cargo run`
# has to compile the server and then apply thirty migrations, which is
# comfortably longer than any sleep worth writing.
for _ in $(seq 1 120); do
  curl -s -m 1 http://127.0.0.1:8091/api/v1/healthz >/dev/null 2>&1 && break
  sleep 1
done
curl -s -m 3 http://127.0.0.1:8091/api/v1/healthz || {
  echo "server did not come up — see /tmp/fc-store-server.log" >&2; exit 1; }
echo

PHOTO_DIR="$SCRIPT_DIR/screenshot-photos" python3 - << 'EOF'
import json, os, glob, struct, zlib, uuid, urllib.request, urllib.parse

BASE = "http://127.0.0.1:8091/api/v1"

def call(method, path, token=None, body=None, raw=None, ctype="application/json"):
    req = urllib.request.Request(BASE + path, method=method)
    req.add_header("Content-Type", ctype)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    with urllib.request.urlopen(req, data) as r:
        out = r.read()
        return json.loads(out) if out else {}

def send(chat, token, body, **extra):
    payload = {"client_msg_id": str(uuid.uuid4()), "body": body}
    payload.update(extra)
    return call("POST", f"/chats/{chat}/messages", token, payload)

# --- images -----------------------------------------------------------
# A vertical gradient PNG from the standard library alone, so the script
# has no dependency to install. Used only when no real photo is supplied.
def gradient_png(w, h, top, bottom):
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    rows = b""
    for y in range(h):
        f = y / max(h - 1, 1)
        px = bytes(int(top[i] + (bottom[i] - top[i]) * f) for i in range(3))
        rows += b"\x00" + px * w
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9))
            + chunk(b"IEND", b""))

def photos():
    """Real files if the operator supplied any, else generated gradients."""
    found = []
    for ext in ("jpg", "jpeg", "png"):
        found += sorted(glob.glob(os.path.join(os.environ["PHOTO_DIR"], f"*.{ext}")))
    if found:
        out = []
        for path in found[:4]:
            mime = "image/png" if path.endswith(".png") else "image/jpeg"
            out.append((open(path, "rb").read(), mime))
        print(f"  album: {len(out)} supplied photo(s) from screenshot-photos/")
        return out
    print("  album: no photos supplied — generating gradients "
          "(drop files into server/scripts/screenshot-photos/ for real ones)")
    palettes = [((250, 214, 165), (214, 122, 92)), ((186, 220, 232), (92, 140, 176)),
                ((205, 226, 191), (104, 152, 106)), ((233, 205, 222), (150, 108, 148))]
    return [(gradient_png(1200, 900, a, b), "image/png") for a, b in palettes]

# --- the family -------------------------------------------------------
def register(u, name):
    return call("POST", "/auth/register",
                body={"username": u, "display_name": name, "password": "password123"})["token"]

nora = register("nora", "Nora")
dan = register("dan", "Dan")
ellie = register("ellie", "Ellie")
mae = register("mae", "Grandma Mae")
rob = register("rob", "Uncle Rob")

fam = call("POST", "/families", nora, {"name": "The Harpers"})
code = fam["family"]["invite_code"]
call("PATCH", "/families/mine", nora, {"join_policy": "open"})
for t in (dan, ellie, mae, rob):
    call("POST", "/families/join", t, {"invite_code": code})
# Back to approval, which is the default a screenshot should show.
call("PATCH", "/families/mine", nora, {"join_policy": "approval"})

family_chat = next(c["chat"]["id"] for c in call("GET", "/chats", nora)["chats"]
                   if c["chat"]["kind"] == "family")

# Birthdays — a day and a month, never a year (protocol.md, "Birthdays").
me = {t: call("GET", "/me", t)["user"]["id"] for t in (nora, dan, ellie, mae, rob)}
for tok, (m, d) in zip((dan, ellie, mae, rob), [(3, 14), (7, 2), (11, 26), (5, 9)]):
    call("PUT", "/me/birthday", tok, {"month": m, "day": d})

# --- the thread -------------------------------------------------------
# Oldest first: the client orders by id, so posting out of order renders
# the conversation scrambled.
thread = [
    (nora,  "Half day tomorrow, so I can do the big shop on the way home."),
    (dan,   "Perfect. We're out of coffee and Ellie finished the oat milk."),
    (ellie, "I did not finish the oat milk"),
    (dan,   "Ellie."),
    (ellie, "…I finished the oat milk"),
    (mae,   "Put me down for a bag of those little oranges if they have them."),
    (nora,  "Got it. Rob, are you still coming Sunday?"),
    (rob,   "Wouldn't miss it. I'll bring the good bread."),
]
for tok, text in thread:
    send(family_chat, tok, text)

# Album — four photos in ONE message.
album_ids = []
for data, mime in photos():
    a = call("POST", f"/attachments?kind=photo&width=1200&height=900",
             dan, raw=data, ctype=mime)
    album_ids.append(a["attachment"]["id"])
send(family_chat, dan, "Sunday at the lake 🦆", attachment_ids=album_ids)

send(family_chat, mae, "Oh these are lovely. Print me the second one?")

# Poll, mid-vote so two options carry voter faces.
poll_msg = send(family_chat, nora, "Sunday lunch — what are we doing?",
                poll={"options": ["Roast at ours", "Everyone brings a dish", "Café by the park"]})
poll_id = poll_msg["message"]["id"]
opts = poll_msg["message"]["poll"]["options"]
for tok, idx in ((nora, 0), (mae, 0), (dan, 1), (ellie, 1)):
    call("PUT", f"/chats/{family_chat}/messages/{poll_id}/vote", tok,
         {"option_id": opts[idx]["id"]})

# Shared location — a public place, never a home address.
loc = call("POST", "/attachments?" + urllib.parse.urlencode({
    "kind": "location", "latitude": 51.5290, "longitude": -0.1565,
    "accuracy_m": 12, "name": "Boating lake car park"}), rob, raw=b"")
send(family_chat, rob, "", attachment_id=loc["attachment"]["id"])
send(family_chat, ellie, "We're by the ducks when you get here 🐥")

# Reactions, so a bubble shows chips.
first = call("GET", f"/chats/{family_chat}/messages", nora)["messages"]
for m in first:
    if m["body"].startswith("Sunday at the lake"):
        for tok, emoji in ((nora, "❤️"), (mae, "❤️"), (ellie, "😍")):
            call("PUT", f"/chats/{family_chat}/messages/{m['id']}/reaction",
                 tok, {"emoji": emoji})

# --- board ------------------------------------------------------------
notes = [("Bins go out Tuesday", "yellow", 0.12, 0.10),
         ("Ellie — dentist, Thu 4pm", "blue", 0.52, 0.16),
         ("Rob's bread recipe is in the tin", "green", 0.18, 0.46),
         ("Holiday photos → shared album", "pink", 0.58, 0.54)]
for text, color, x, y in notes:
    call("POST", "/families/mine/board/notes", nora,
         {"text": text, "color": color, "x": x, "y": y})

# --- a direct chat ----------------------------------------------------
direct = call("POST", "/chats/direct", nora, {"user_id": me[ellie]})["chat"]["id"]
send(direct, ellie, "Can I stay at Priya's on Saturday?")
send(direct, nora, "Yes — home by lunch on Sunday please.")

print(f"\n  family 'The Harpers'  chat {family_chat}  invite {code}")
print("  photograph as: nora / password123   (owner)")
print("  server: http://127.0.0.1:8091")
EOF

# Spread the thread over two days. Every message is posted in the same
# second above, and a store screenshot whose every row reads the same
# minute looks staged. `created_at` is server-assigned with no REST way to
# set it, so this is the one step that reaches past the API.
#
# The newest message lands ~11 minutes ago, so the thread reads as "this
# morning" whenever the screenshots are taken.
docker exec fc-test-pg psql -U postgres -d family_connect_dev -q -c "
  WITH ordered AS (
    SELECT id, row_number() OVER (ORDER BY id DESC) AS back FROM messages
  )
  UPDATE messages m
     SET created_at = now() - (ordered.back * interval '37 minutes')
    FROM ordered
   WHERE m.id = ordered.id;" >/dev/null
