#!/usr/bin/env python3
"""Is the App Review demo family still in the state the notes describe?

    FC_OWNER_PASS=… FC_MEMBER_PASS=… server/scripts/check-review-family.py \
        --server https://fc.nettrash.me --owner TEST_1 --member TEST_2 \
        --processor "Microsoft — Azure OpenAI (Sweden Central)"

Read-only, and every check is a sentence from the review notes. Submission
2546d5da was rejected under 2.1(a) because the notes had drifted from the
server: the member account had blocked the owner (so the one-to-one chat,
and with it every call, was unreachable from that account), and the family
held test junk instead of the content the notes promised. None of that is
visible from the repo, and all of it is visible from here.

Run it before filing, and again while a review is open — a single tap in
the app can undo any of it.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from importlib import import_module

seed = import_module("seed-review-family".replace("-", "_")) if False else None  # noqa: E731


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="https://fc.nettrash.me")
    ap.add_argument("--owner", default="TEST_1")
    ap.add_argument("--member", default="TEST_2")
    ap.add_argument(
        "--processor",
        help="the exact string the review notes quote; compared with what the "
        "server advertises, because the app prints it verbatim",
    )
    args = ap.parse_args()

    owner_pass = os.environ.get("FC_OWNER_PASS")
    member_pass = os.environ.get("FC_MEMBER_PASS")
    if not owner_pass or not member_pass:
        print("set FC_OWNER_PASS and FC_MEMBER_PASS", file=sys.stderr)
        return 2

    api = Api(args.server.rstrip("/") + "/api/v1")
    problems: list[str] = []

    def check(ok: bool, good: str, bad: str) -> None:
        print(("  ok   " if ok else "  FAIL ") + (good if ok else bad))
        if not ok:
            problems.append(bad)

    tokens = {}
    for role, username, password in (
        ("owner", args.owner, owner_pass),
        ("member", args.member, member_pass),
    ):
        try:
            tokens[role] = api.login(username, password)
        except SystemExit as exit_:
            print(f"  FAIL {username} cannot sign in: {exit_}")
            return 1
    print(f"both accounts sign in on {args.server}")

    me = {role: api.get("/me", token) for role, token in tokens.items()}
    ids = {role: value["user"]["id"] for role, value in me.items()}

    # The 2.1(a) cause, first: a block hides the pair's chat from the
    # blocker and refuses to open it, and calls exist nowhere else.
    for role, other in (("owner", "member"), ("member", "owner")):
        blocked = me[role]["blocked_user_ids"]
        check(
            ids[other] not in blocked,
            f"{role} has not blocked the {other}",
            f"the {role} HAS BLOCKED the {other} — the one-to-one chat, and every "
            "call, is unreachable from the blocker's device",
        )

    for role, token in tokens.items():
        chats = {
            entry["chat"]["kind"]: entry["chat"]
            for entry in api.get("/chats", token)["chats"]
        }
        check(
            "direct" in chats,
            f"the {role} sees the one-to-one chat (calls are placed there)",
            f"the {role} has NO one-to-one chat — the reviewer cannot place a call",
        )
        check("family" in chats, f"the {role} sees the family chat", f"the {role} has no family chat")
        check("ai" in chats, f"the {role} sees the assistant chat", f"the {role} has no assistant chat")

    # The consent screen the notes send them to, and the state after it.
    check(
        me["owner"]["assistant_consent_at"] is None,
        f"{args.owner} has NOT agreed — the consent screen is what they meet",
        f"{args.owner} HAS agreed ({me['owner']['assistant_consent_at']}) — the reviewer "
        "will never see the consent screen the notes describe",
    )
    check(
        me["member"]["assistant_consent_at"] is not None,
        f"{args.member} has agreed, so the state after consent is visible too",
        f"{args.member} has not agreed — the 'after' half of the disclosure is missing",
    )

    family = api.get("/families/mine", tokens["owner"])
    assistant = family.get("assistant")
    check(assistant is not None, "the family advertises an assistant", "NO assistant is advertised — "
          "check `[ai] processor` in the live config; without it the feature disappears")
    if assistant:
        named = (assistant.get("processor") or "").strip()
        check(bool(named), f"it names a processor: {named!r}", "the assistant names NO processor")
        if args.processor:
            check(
                named == args.processor.strip(),
                "the notes and the server quote the same processor",
                f"the notes say {args.processor!r} and the server says {named!r} — the app "
                "prints the server's string verbatim on the consent screen",
            )

    check(me["owner"].get("calls_enabled") is True, "the server signals calls", "calls are OFF on this server")
    check(
        me["owner"].get("video_calls_enabled") is True,
        "and video calls",
        "video calls are OFF, and the listing claims them",
    )
    check(
        bool(me["owner"].get("support_contact")),
        "a support contact is published (the report screen needs it)",
        "no support_contact is set — the report sheet draws no escalation line",
    )

    # The content the notes promise, in the newest slice a reviewer sees.
    family_chat = next(
        entry["chat"]["id"]
        for entry in api.get("/chats", tokens["owner"])["chats"]
        if entry["chat"]["kind"] == "family"
    )
    messages = api.get(f"/chats/{family_chat}/messages?limit=30", tokens["owner"])["messages"]
    kinds: dict[str, int] = {}
    polls = 0
    for message in messages:
        for attachment in message.get("attachments") or []:
            kinds[attachment["kind"]] = kinds.get(attachment["kind"], 0) + 1
        if message.get("poll"):
            polls += 1
    check(kinds.get("photo", 0) >= 2, f"photos in the newest 30 messages ({kinds.get('photo', 0)})", "no photo album near the top of the family chat")
    check(kinds.get("audio", 0) >= 1, "a voice note", "no voice note in the newest 30 messages")
    check(kinds.get("location", 0) >= 1, "a shared location", "no shared location in the newest 30 messages")
    check(polls >= 1, f"an open poll ({polls})", "no poll in the newest 30 messages")

    notes = api.get("/families/mine/board", tokens["owner"])["notes"]
    board_kinds = {note.get("kind") for note in notes}
    check("event" in board_kinds, "the board carries an event", "the board has no event card")
    check("tasks" in board_kinds, "and a task list", "the board has no task list")
    check(len(notes) <= 20, f"the board is readable ({len(notes)} notes)", f"the board holds {len(notes)} notes — a reviewer opens a wall of them")

    print()
    if problems:
        print(f"{len(problems)} problem(s) a reviewer would meet:")
        for problem in problems:
            print("  -", problem)
        return 1
    print("the demo family matches the review notes.")
    return 0


class Api:
    def __init__(self, base: str) -> None:
        self.base = base
        import ssl

        self.context = (
            ssl.create_default_context(cafile="/etc/ssl/cert.pem")
            if os.path.exists("/etc/ssl/cert.pem")
            else ssl.create_default_context()
        )

    def call(self, method, path, token=None, body=None):
        import json
        import urllib.error
        import urllib.request

        request = urllib.request.Request(self.base + path, method=method)
        request.add_header("Content-Type", "application/json")
        if token:
            request.add_header("Authorization", f"Bearer {token}")
        data = json.dumps(body).encode() if body is not None else None
        try:
            with urllib.request.urlopen(request, data, context=self.context) as answer:
                out = answer.read()
                return json.loads(out) if out else {}
        except urllib.error.HTTPError as error:
            raise SystemExit(f"{method} {path} → {error.code}") from error

    def login(self, username, password):
        # In the BODY, never a URL: a password in a query string lands in logs.
        return self.call("POST", "/auth/login", body={"username": username, "password": password})["token"]

    def get(self, path, token):
        return self.call("GET", path, token)


if __name__ == "__main__":
    sys.exit(main())
