# Emoji as you type (issue #101)

**Written 2026-10-18** against `v1.3`, in answer to issue #101: *"We should replace text combinations with emoji
symbols. For example: :) should be replaced by 😄 and so on. The best way is to just copy regular approach for
example from Telegram."*

Owner's choices (2026-10-18): Telegram's emoticons plus their no-nose forms (so `:)` works); `:name:` shortcodes too,
replaced on the closing colon, with no suggestion popup; replaced as you type, with Backspace to take it back; always
on, no setting.

The wire does not change: what is sent is the emoji itself, so every client — and the server, and a client from
before #101 — sees the same message. Nothing here is in `docs/protocol.md`.

## 1. What Telegram does, and what this copies

Telegram Desktop (lib_ui `InputField::processInstantReplaces`, desktop-app/codegen `emoji/data.cpp` and
`emoji/replaces.cpp`):

- It replaces the moment the last character of a combination is typed — one typed character at a time, so never a
  paste — and never inside code.
- A Backspace straight after puts back what was typed.
- Its emoticon table has 27 entries, every face with a nose (`:-)`, not `:)`), plus `:o ` and `xD `, which wait for a
  space because `:o` starts `:ok_hand:` and `xD` starts `xDDD`.
- It turns about 4,000 `:name:` shortcodes into emoji: JoyPixels' names, plus `:like:` `:dislike:` `:hmm:` `:party:`.
- Its mobile apps do neither.

This copies all of that, on every client, phones included, with three differences:

1. **The no-nose forms** (the owner's choice): `:)` `:D` `;)` `:(` `:*` `:P` `;P` `;p`, `:p ` (it waits for a space:
   `:pizza:`), and the case twins Telegram's case-sensitive table misses (`:-P`, `;-p`, `O:-)`, `}:-)`, `XD `). `8)`
   and `B)` are left out on purpose — "plan B)" and a list's "8)" are far commoner than either face.
2. **Guards.** Telegram replaces a combination wherever it ends, so `item 3(b)` loses its `3(` to 😔 and `x<3` its
   `<3` to ❤️. Here a face that starts with `:` or `;` must not follow an ASCII digit (`a[1:]`, `1:)`), and any other
   must not follow an ASCII letter or digit (`x<3`, `planB-)`). A shortcode's opening colon must not follow an ASCII
   letter or digit either (`10:30:`, `a:b:`). ASCII on purpose: the four languages' "is a letter" disagree past it.
3. **Growing.** In Telegram `>(` is replaced before the second `(` of `>((` can be typed, so 😡 is unreachable. Here a
   replacement just made grows when the next key makes a longer face from the same place: `:(` then `(` is 😩, and its
   Backspace puts back all three characters.

A space-ended emoticon keeps its space (`:o ` becomes `😨 `), where Telegram swallows it.

The shortcode names come from JoyPixels' `emoji.json` (emoji-toolkit 11.0.0, MIT), the source of Telegram's own
list, rather than Telegram's copy (GPL). `:shrug:` is therefore 🤷, JoyPixels' meaning, not Telegram's `¯\_(ツ)_/¯`,
whose backslash and underscores are markdown here.

## 2. The rule

`replace(before, typed, live)` — `before` is the field's text before the caret, without the typed character;
`typed` is the one character the keyboard just produced; `live` is the swap the field still holds, if any. It
returns a **swap** — where the replacement starts, what is put in, and everything that was typed — or nothing, and
the character goes in as it is. In order:

1. **Not in code**: the caret is not inside a fence or a code span, by #97's test (`in_code`).
2. **Growing**: the caret is right behind `live`'s replacement, and `live`'s typed text plus `typed` is an emoticon.
   The new swap starts where `live` did.
3. **A shortcode**, when `typed` is `:` — `before` ends in `:name`, the name is non-empty with no whitespace and no
   colon in it, the opening colon is at the start or after something that is not an ASCII letter or digit, and the
   name is in the table (exactly: `:Fire:` is not `:fire:`).
4. **An emoticon**: `before` plus `typed` ends in one, and the longest whose guard passes wins. `HELLO:)` is `:)`
   after `HELL`'s `O`, because the longer `O:)` fails its guard.

An emoticon that ends in a space puts the space back after the emoji.

**Taking it back.** The field keeps the swap from the moment the rule makes it until anything else happens — a key
the rule does not act on, a click, a caret move, a paste, a selection. A Backspace with the caret right behind the
replacement and nothing selected puts back everything that was typed (`:-)`, `:fire:`, `:o `) instead of deleting.
The same as #97's capital, and like it, the swap is dropped after.

### The emoticons

| Typed | Emoji | | Typed | Emoji |
|---|---|---|---|---|
| `:-)` `:)` | 😊 | | `:'(` | 😢 |
| `8-)` | 😍 | | `:_(` | 😭 |
| `<3` | ❤️ | | `:((` | 😩 |
| `:-*` `:*` | 😚 | | `:\|` | 😐 |
| `B-)` | 😎 | | `3-)` | 😌 |
| `:-D` `:D` | 😃 | | `>(` | 😠 |
| `;-)` `;)` | 😉 | | `>((` | 😡 |
| `;-P` `;-p` `;P` `;p` | 😜 | | `O:)` `O:-)` | 😇 |
| `:-p` `:-P` `:P` `:p␣` | 😋 | | `;o` | 😰 |
| `3(` | 😔 | | `8\|` | 😳 |
| `:-(` `:(` | 😞 | | `8o` | 😲 |
| `:]` | 😏 | | `:X` | 😷 |
| `:o␣` | 😨 | | `}:)` `}:-)` | 😈 |
| `xD␣` `XD␣` | 😆 | | | |

`␣` is the space the entry waits for. `XD␣` is there because a desktop client capitalises the `x` of `xD` at the
start of a sentence (#97).

### The shortcodes

`web/text/data/emoji-names.tsv` — 6,552 names: every `shortname` and `shortname_alternates` entry of JoyPixels'
`emoji.json`, plus Telegram's four. One line per name (`fire⇥🔥`), `#` lines are comments; the file carries
JoyPixels' MIT notice. Regenerate it, and the three copies, with `tools/emoji/make_names.py` (its docstring has the
download line); never edit it by hand.

| Copy | Read by |
|---|---|
| `web/text/data/emoji-names.tsv` | the original; `fc_text::emoji_typing` (`include_str!`) |
| `ios/FamilyConnect/Emoji/emoji-names.tsv` | iOS and Mac, from the app bundle |
| `android/app/src/main/resources/emoji-names.tsv` | Android, a Java resource (so JVM tests read it too) |
| `win/src/FamilyConnect.App.Logic/emoji-names.tsv` | Windows, an embedded resource |

CI's "One set of rich files and emoji names" job fails if a copy differs.

## 3. Where it runs

On every **prose field** (#97's list: the chat composer, which is also where a message is edited, the thread
reply, a board note or caption, a list's line, an event's title, a poll's question and options, a report's note),
on every client, single-line fields included — unlike #97's desktop capitals, which only the multi-line fields get.
Never a username, a server address, a code, a search field or a sticker label.

| Client | How the typed character is seen |
|---|---|
| Web | `beforeinput` with `insertText` of one character, on every device (the capitals' handler, extended) |
| Mac | the capitals' `NSTextView` hook (`MacCapitalKeys`), extended to every prose field |
| Windows | the capitals' `TextBox` hook (`CapitalLetters`), extended to every prose field |
| iPhone / iPad | the field's text binding: a change that is exactly one character added at the end |
| Android | the field's value: a change that is exactly one character inserted at the caret |

A phone's keyboard gives no key events, so iOS and Android judge by what changed: one character more, nothing
else different. A paste, a dictation, a suggestion or an autocorrection changes more than that, and is left alone.
On iOS, where SwiftUI's field cannot tell where the caret is, only an addition at the end counts.

Capitals (#97) run first, on the desktop: the typed character the emoji rule sees is the one capitals decided.

### Where it lives

| Port | Rule | Tests |
|---|---|---|
| Rust (web) | `web/text/src/emoji_typing.rs` | the table below, in that file's tests |
| Swift (iOS, Mac) | `ios/FamilyConnect/Core/EmojiTyping.swift` | `ios/FamilyConnectTests/EmojiTypingTests.swift` |
| Kotlin (Android) | `android/app/src/main/java/me/nettrash/familyconnect/util/EmojiTyping.kt` | `EmojiTypingTest.kt` |
| C# (Windows) | `win/src/FamilyConnect.App.Logic/EmojiTyping.cs` | `win/tests/FamilyConnect.App.Logic.Tests/EmojiTypingTests.cs` |

Every port is held to `emoji-typing-vectors.json`, which board-oracle prints from the Rust (`cargo run --quiet --
emoji-typing`): the emoticon table, and 1,600 key sequences typed into an empty field with what it then holds — the
table below, every emoticon behind every kind of character the guards tell apart and taken back, and a spread of the
shortcode names. The file lives beside the C# tests, in the iOS test bundle and on Android's JVM test classpath; CI
prints it again and compares all three.

In the keys, `⌫` is a Backspace and `→` is "something else happened" (the live swap is dropped). An ordinary
Backspace removes one character; the vectors only press one after ASCII.

## 4. The table every port's tests hold

`fc_text::emoji_typing::TYPING_TABLE`, typed into an empty field (`⏎` is a line feed):

| Keys | Field | Why |
|---|---|---|
| `:-)` | 😊 | Telegram's |
| `:)` | 😊 | the no-nose form |
| `:o␣` | `😨␣` | waits for the space, keeps it |
| `:o` | `:o` | no space yet |
| `:ok_hand:` | 👌 | so `:o` cannot eat it |
| `Hi :)` | `Hi 😊` | |
| `thanks:)` | `thanks😊` | a letter before `:` is fine |
| `1:]` | `1:]` | a digit before `:` is not |
| `a[1:]` | `a[1:]` | |
| `x<3` | `x<3` | a letter before `<` is not |
| `i <3 u` | `i ❤️ u` | |
| `item3(` | `item3(` | |
| `plan B-)` | `plan 😎` | |
| `HELLO:)` | `HELLO😊` | `O:)` fails its guard, `:)` passes |
| `hi O:)` | `hi 😇` | |
| `:((` | 😩 | grown from `:(` |
| `>((` | 😡 | |
| `:((⌫` | `:((` | one Backspace puts back all three |
| `:(→(` | `😞(` | something else happened: no growing |
| `:fire:` | 🔥 | |
| `:+1:` | 👍 | |
| `:piñata:` | 🪅 | |
| `:like:` | 👍 | Telegram's own |
| `(:fire:)` | `(🔥)` | |
| `::fire:` | `:🔥` | |
| `:Fire:` | `:Fire:` | names are exact |
| `a:fire:` | `a:fire:` | a letter before the opening colon |
| `10:100:` | `10:100:` | a digit before it |
| `:fire :` | `:fire :` | no spaces in a name |
| `:fire::)` | `🔥😊` | |
| `:)⌫` | `:)` | taken back |
| `:fire:⌫` | `:fire:` | |
| `:)⌫⌫` | `:` | the second Backspace is ordinary |
| `:)→⌫` | `` | after something else, an ordinary Backspace |
| `` `:) `` | `` `:) `` | in a code span |
| `` ```⏎:fire: `` | unchanged | in a fence |

The full table (99 rows, every emoticon among them) is in the Rust source; the vectors carry it.
