#!/usr/bin/env python3
"""Make the App Review demo family look like a family, against a LIVE server.

    FC_OWNER_PASS=… FC_MEMBER_PASS=… server/scripts/seed-review-family.py \
        --server https://fc.nettrash.me --owner TEST_1 --member TEST_2

WHY THIS EXISTS. `seed-store-screenshots.sh` builds the same kind of fixture,
but it drops and recreates a local database first and registers its own
five-person family — none of which is possible or wanted on the server App
Review actually signs in to. Submission 2546d5da was rejected under guideline
2.1(a) for exactly the gap: the review notes promised photos, a poll and a
voice note, and the family held 253 messages of "jaijai", 119 board notes
reading "Test Note", and a member account that had BLOCKED the owner — so the
second device had no one-to-one chat, and therefore no calls, no read
receipts and no typing indicators to verify.

WHAT IT DOES, in order:

  1. Unblocks in both directions. This is the one that broke the review.
  2. Deletes every board note, then posts a wall worth looking at.
  3. Posts a conversation, an album, a voice note, a location, two open polls
     mid-vote and reactions, plus a one-to-one exchange.
  4. Leaves the assistant where the notes say it is: the OWNER has not agreed
     (so the reviewer meets the consent screen on their first message) and the
     MEMBER has, with one exchange behind it.

WHAT IT CANNOT DO. This protocol has no message deletion — by design — so the
junk already in the chat stays. It does not matter much: a chat opens at its
newest message, and everything posted here lands after it. To remove it
properly, run the SQL in the block this script prints at the end, on the box,
after a backup.

Nothing here resembles a real person. Every name, message and photograph is
invented or generated, because this content ends up in front of a reviewer.
"""

import argparse
import json
import os
import struct
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid
import wave
import zlib
from datetime import datetime, timedelta


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="https://fc.nettrash.me")
    ap.add_argument("--owner", default="TEST_1")
    ap.add_argument("--member", default="TEST_2")
    ap.add_argument(
        "--ask-assistant",
        action="store_true",
        help="have the member ask the assistant one question, so its chat has "
        "an exchange behind it. Costs tokens and needs the deployment up.",
    )
    args = ap.parse_args()

    owner_pass = os.environ.get("FC_OWNER_PASS")
    member_pass = os.environ.get("FC_MEMBER_PASS")
    if not owner_pass or not member_pass:
        print("set FC_OWNER_PASS and FC_MEMBER_PASS in the environment", file=sys.stderr)
        return 2

    api = Api(args.server.rstrip("/") + "/api/v1")
    owner = api.login(args.owner, owner_pass)
    member = api.login(args.member, member_pass)
    print(f"signed in: {args.owner} (owner) and {args.member} (member)")

    owner_id = api.get("/me", owner)["user"]["id"]
    member_id = api.get("/me", member)["user"]["id"]

    # 1. THE BLOCK. Both directions, because either one hides the pair's chat
    #    from the blocker and refuses to open it (`blocked`, 409) — and calls
    #    exist only inside that chat.
    for token, other, who in ((owner, member_id, args.owner), (member, owner_id, args.member)):
        before = api.get("/me", token)["blocked_user_ids"]
        if other in before:
            api.delete(f"/families/members/{other}/block", token)
            print(f"  unblocked: {who} had blocked the other account")
    print("  blocks: none in either direction")

    # 2. THE WALL. Cleared and rebuilt: 119 notes reading "Test Note" is what
    #    a reviewer opens the board to.
    #    Each note goes with ITS AUTHOR's token: `delete_note` answers
    #    `not_note_author` to anybody else, the owner included (the board is
    #    the family's, but a note is its author's).
    tokens = {owner_id: owner, member_id: member}
    notes = api.get("/families/mine/board", owner).get("notes", [])
    deleted, stranded = 0, 0
    for note in notes:
        token = tokens.get(note.get("author_id"))
        if token is None:
            stranded += 1
            continue
        api.delete(f"/families/mine/board/notes/{note['id']}", token)
        deleted += 1
    print(f"  board: deleted {deleted} note(s)" + (
        f"; {stranded} left, authored by somebody neither account can delete for" if stranded else ""))

    family_chat = next(
        entry["chat"]["id"]
        for entry in api.get("/chats", owner)["chats"]
        if entry["chat"]["kind"] == "family"
    )

    # 3. A CONVERSATION. Oldest first: clients order by id, so posting out of
    #    order renders it scrambled.
    for token, text in (
        (owner, "Half day tomorrow, so I can do the big shop on the way home."),
        (member, "Perfect. We're out of coffee and the oat milk is gone."),
        (owner, "Noted. Are you still coming on Sunday?"),
        (member, "Wouldn't miss it. I'll bring the good bread."),
    ):
        api.send(family_chat, token, text)

    root = next(
        m["id"]
        for m in api.get(f"/chats/{family_chat}/messages", owner)["messages"]
        if "still coming on Sunday" in (m["body"] or "")
    )
    for token, text in (
        (member, "Sunday works. What time do you want us?"),
        (owner, "One-ish. I'll start the potatoes at noon."),
        (member, "I'll do the bread and a pudding then."),
        (owner, "Perfect. That makes six, so we'll use the long table."),
        (member, "I'll get the leaf out of the loft on Saturday."),
    ):
        api.send(family_chat, token, text, reply_to_message_id=root)
    print("  chat: a thread and a five-reply chain")

    # An album — four photographs in ONE message.
    album = [
        api.upload(
            "/attachments?kind=photo&width=1200&height=900", member, png, "image/png"
        )
        for png in (
            gradient_png(1200, 900, (250, 214, 165), (214, 122, 92)),
            gradient_png(1200, 900, (186, 220, 232), (92, 140, 176)),
            gradient_png(1200, 900, (205, 226, 191), (104, 152, 106)),
            gradient_png(1200, 900, (233, 205, 222), (150, 108, 148)),
        )
    ]
    api.send(family_chat, member, "Sunday at the lake 🦆", attachment_ids=album)
    api.send(family_chat, owner, "Oh these are lovely. Print me the second one?")

    # A voice note. Generated, and a WAV because that is a header this script
    # can write honestly; the app records m4a and the server takes either.
    voice = api.upload(
        "/attachments?" + urllib.parse.urlencode({"kind": "audio", "duration_ms": 3200}),
        owner,
        spoken_wav(3.2),
        "audio/wav",
    )
    api.send(family_chat, owner, "", attachment_id=voice)

    # A shared location — a public place, never a home address.
    pin = api.upload(
        "/attachments?"
        + urllib.parse.urlencode(
            {
                "kind": "location",
                "latitude": 51.5290,
                "longitude": -0.1565,
                "accuracy_m": 12,
                "name": "Boating lake car park",
            }
        ),
        member,
        b"",
        "application/octet-stream",
    )
    api.send(family_chat, member, "", attachment_id=pin)
    api.send(family_chat, owner, "We're by the ducks when you get here 🐥")
    print("  chat: an album, a voice note and a location")

    # Two open polls, both mid-vote: Open Polls is a LIST, and one card on it
    # reads as an empty screen.
    for asker, question, options, votes in (
        (owner, "Sunday lunch — what are we doing?",
         ["Roast at ours", "Everyone brings a dish", "Café by the park"], ((owner, 0), (member, 1))),
        (member, "Who can do the school run on Friday?",
         ["I can", "You", "Ask your brother"], ((member, 0), (owner, 1))),
    ):
        posted = api.send(family_chat, asker, question, poll={"options": options})["message"]
        ids = [o["id"] for o in posted["poll"]["options"]]
        for token, index in votes:
            api.put(
                f"/chats/{family_chat}/messages/{posted['id']}/vote",
                token,
                {"option_id": ids[index]},
            )
    print("  chat: two open polls, mid-vote")

    for message in api.get(f"/chats/{family_chat}/messages", owner)["messages"]:
        if (message["body"] or "").startswith("Sunday at the lake"):
            for token, emoji in ((owner, "❤️"), (member, "😍")):
                api.put(
                    f"/chats/{family_chat}/messages/{message['id']}/reaction",
                    token,
                    {"emoji": emoji},
                )
            break

    # The wall: plain notes, an event with answers, and a task list part-ticked.
    for text, color, x, y in (
        ("Bins go out Tuesday", "yellow", 0.06, 0.04),
        ("Dentist, Thursday 4pm", "blue", 0.60, 0.05),
        ("Bread recipe is in the tin", "green", 0.14, 0.78),
        ("Holiday photos → shared album", "pink", 0.62, 0.84),
    ):
        api.post("/families/mine/board/notes", owner, {"text": text, "color": color, "x": x, "y": y})

    local = datetime.now().astimezone().tzinfo
    one = datetime.now(local).replace(hour=13, minute=0, second=0, microsecond=0)
    starts = one + timedelta(days=(6 - one.weekday()) % 7 or 7)
    event = api.post(
        "/families/mine/board/notes",
        owner,
        {
            "text": "Sunday lunch at ours", "color": "blue", "x": 0.06, "y": 0.22,
            "kind": "event", "size": "large", "place": "Ours",
            "starts_at": stamp(starts), "ends_at": stamp(starts + timedelta(hours=3)),
        },
    )["note"]["id"]
    api.put(f"/families/mine/board/notes/{event}/rsvp", member, {"answer": "going"})

    tasks = api.post(
        "/families/mine/board/notes",
        owner,
        {
            "text": "Before Sunday", "color": "green", "x": 0.54, "y": 0.40,
            "kind": "tasks", "size": "large",
            "items": [{"text": "Order the meat"}, {"text": "Borrow chairs"},
                      {"text": "Charge the camera"}, {"text": "Ice for drinks"}],
        },
    )["note"]
    for item in tasks["items"][:2]:
        api.put(
            f"/families/mine/board/notes/{tasks['id']}/tasks/{item['id']}", member, {"done": True}
        )
    print("  board: four notes, an event with an answer, a task list mid-tick")

    # The one-to-one chat the notes send the reviewer to on a second device.
    direct = next(
        (
            entry["chat"]["id"]
            for entry in api.get("/chats", owner)["chats"]
            if entry["chat"]["kind"] == "direct" and entry["chat"].get("peer_user_id") == member_id
        ),
        None,
    )
    if direct is None:
        direct = api.post("/chats/direct", owner, {"user_id": member_id})["chat"]["id"]
    api.send(direct, owner, "Did the bread tin ever come back?")
    api.send(direct, member, "It's in the car. I'll bring it Sunday.")
    api.send(direct, owner, "Thank you. Bring the big one if you can find it.")
    print(f"  direct chat {direct}: an exchange both accounts can see")

    # 4. THE ASSISTANT, exactly as the review notes describe it.
    api.post("/me/assistant-consent", member, {"granted": True})
    if args.ask_assistant:
        ai = next(
            entry["chat"]["id"]
            for entry in api.get("/chats", member)["chats"]
            if entry["chat"]["kind"] == "ai"
        )
        api.send(ai, member, "In one sentence: what should I bring to a family lunch?")
        print("  assistant: the member agreed and asked one question")
    else:
        print("  assistant: the member agreed (pass --ask-assistant for an exchange)")
    api.post("/me/assistant-consent", owner, {"granted": False})
    consent = api.get("/me", owner)["assistant_consent_at"]
    assert consent is None, f"the owner must NOT have agreed, got {consent!r}"
    print("  assistant: the owner has NOT agreed — the consent screen is what they meet")

    print("\nthe junk this cannot reach, and the SQL that can (run on the box, after a backup):")
    print(f"""
  sudo -u postgres psql family_connect -c "
    BEGIN;
    DELETE FROM messages WHERE chat_id = {family_chat} AND id < {root};
    COMMIT;"
""")
    print("  everything posted above is NEWER than that cut, so the chat still reads correctly.")
    return 0


def stamp(when: datetime) -> str:
    return when.isoformat().replace("+00:00", "Z")


def gradient_png(width: int, height: int, top, bottom) -> bytes:
    """A vertical gradient, from the standard library alone."""

    def chunk(tag: bytes, data: bytes) -> bytes:
        body = struct.pack(">I", len(data)) + tag + data
        return body + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    rows = b""
    for y in range(height):
        fraction = y / max(height - 1, 1)
        pixel = bytes(int(top[i] + (bottom[i] - top[i]) * fraction) for i in range(3))
        rows += b"\x00" + pixel * width
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def spoken_wav(seconds: float) -> bytes:
    """A quiet, gently wavering tone: something to play, and obviously not a voice."""
    import io
    import math

    rate = 16000
    frames = bytearray()
    for n in range(int(rate * seconds)):
        t = n / rate
        envelope = min(1.0, t * 4, (seconds - t) * 4)
        sample = 0.18 * envelope * math.sin(2 * math.pi * (196 + 8 * math.sin(t * 2)) * t)
        frames += struct.pack("<h", int(sample * 32767))
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(bytes(frames))
    return buffer.getvalue()


def https_context():
    """A verifying TLS context, whichever CA bundle this Python can find.

    A python.org build ships no trust store of its own, so the default context
    fails with CERTIFICATE_VERIFY_FAILED against a perfectly good certificate.
    Verification is never disabled here — this request carries a password.
    """
    import ssl

    for where in (_certifi(), "/etc/ssl/cert.pem"):
        if where and os.path.exists(where):
            return ssl.create_default_context(cafile=where)
    return ssl.create_default_context()


def _certifi():
    try:
        import certifi

        return certifi.where()
    except ImportError:
        return None


class Api:
    def __init__(self, base: str) -> None:
        self.base = base
        self.context = https_context()

    def call(self, method, path, token=None, body=None, raw=None, ctype="application/json"):
        request = urllib.request.Request(self.base + path, method=method)
        request.add_header("Content-Type", ctype)
        if token:
            request.add_header("Authorization", f"Bearer {token}")
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        try:
            with urllib.request.urlopen(request, data, context=self.context) as answer:
                out = answer.read()
                return json.loads(out) if out else {}
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")[:300]
            raise SystemExit(f"{method} {path} → {error.code} {detail}") from error

    def login(self, username, password):
        # The password goes in the BODY. Never in a URL: it would land in
        # access logs and in anything that proxies this.
        return self.call(
            "POST", "/auth/login", body={"username": username, "password": password}
        )["token"]

    def get(self, path, token):
        return self.call("GET", path, token)

    def post(self, path, token, body):
        return self.call("POST", path, token, body)

    def put(self, path, token, body):
        return self.call("PUT", path, token, body)

    def delete(self, path, token):
        return self.call("DELETE", path, token)

    def upload(self, path, token, data, ctype):
        return self.call("POST", path, token, raw=data, ctype=ctype)["attachment"]["id"]

    def send(self, chat, token, body, **extra):
        payload = {"client_msg_id": str(uuid.uuid4()), "body": body}
        payload.update(extra)
        return self.call("POST", f"/chats/{chat}/messages", token, payload)


if __name__ == "__main__":
    sys.exit(main())
