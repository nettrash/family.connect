# Capital letters, formulas, diagrams and graphs (issue #97)

**Written 2026-10-17** against `v1.3`, in answer to issue #97: *"For all clients we should support capital letters
automation. Also we need to extend support of markdown by schemes, diagrams, graphs, formulas."*

Owner's choices (2026-10-17): KaTeX + Mermaid + Graphviz + md's plotter, not PlantUML (GPL, 7.4 MB); formulas both as
blocks and inline; desktop capitalisation built in and always on.

The wire does not change. What a message MEANS is in `docs/protocol.md`, "Rich blocks: formulas, diagrams and
graphs" — the parsing rules there are the contract the four parsers are held to. This file is how the clients do it.

## 1. Capital letters

### Who already has them

| Client | Before #97 | After |
|---|---|---|
| iPhone / iPad | SwiftUI's default: the keyboard capitalises sentences | the same, said explicitly on every prose field |
| Android | no field asked, so most keyboards did not | `KeyboardCapitalization.Sentences` on every prose field |
| Web on a phone | the browser's default | `autocapitalize="sentences"` on every prose field |
| Mac | none (no keyboard does it) | the rule below, while the system's "Capitalise words automatically" is on |
| Windows | none | the rule below |
| Web on a computer | none | the rule below, on devices with a fine pointer and hover |

"Prose fields" are the ones people write sentences in: the chat composer (which is also where a message is
edited), the thread reply, a board note or caption, a list's line, an event's title, a poll's question and options,
and a report's note. Never a username, a server address, a code, a search field or a sticker label.

On the desktop the rule runs on MULTI-LINE fields — the composer, the thread reply, a note, a report's note — which
is how the Mac tells them apart (`MacNewLineKeys.takesLines`) and the same set on every desktop client.

### The rule

`capitalizes(before, typed)` — `before` is the text before the caret (the selection's start), `typed` is the one
character the keyboard produced. It answers YES only when all of these hold:

1. `typed` is a lower-case letter whose upper case (invariant, not the locale's) is exactly one character. `ß` is
   left alone.
2. The word is just starting: after setting aside any opening marks typed just before it (`"`, `'`, `(`, `[`, `«`,
   `“`, `‘`), `before` is empty or ends in a space, a tab or a line break.
3. A sentence is just starting. Take `before` without those opening marks and without trailing spaces and tabs;
   it is a sentence start when it
   - is empty, or ends in a line break, or
   - is, on its current line, nothing but a list or heading or quote marker: `-`, `*`, `+`, `>`, one to six `#`, or
     digits followed by `.` or `)`; or
   - ends in `.`, `!` or `?`, optionally followed by closing marks (`"`, `'`, `)`, `]`, `»`, `”`, `’`) — except
     that `.` does not count when the character before it is also `.` (an ellipsis trails off), or when the word it
     ends is an abbreviation: the word back to the previous space contains another `.` and every piece between the
     dots is one or two ASCII letters (`e.g.`, `i.e.`, `a.m.`, `U.S.`).
4. The caret is not in code: `before` has an even number of fence lines (lines starting with three backticks), and
   its current line, outside a fence, has an even number of backticks.

**Taking it back.** When the rule has just capitalised a letter, a Backspace pressed straight after it — caret right
behind that letter, nothing selected — puts the lower-case letter back instead of deleting it. Anything else
(another key, a click, a paste) forgets that it can.

### Where it lives

| Port | Rule | Tests |
|---|---|---|
| Rust (web) | `web/text/src/composer.rs` `capitalizes` | the table below, in that file's tests |
| Swift (Mac; iOS compiles it too) | `ios/FamilyConnect/Core/Capitals.swift` | `ios/FamilyConnectTests/CapitalsTests.swift` |
| C# (Windows) | `win/src/FamilyConnect.App.Logic/Capitals.cs` | `win/tests/FamilyConnect.App.Logic.Tests/CapitalsTests.cs` |

Android does not need the rule: its keyboards apply their own.

### The table every port's tests hold

| before | typed | capitalises |
|---|---|---|
| `` (empty) | `h` | yes |
| `Hi. ` | `h` | yes |
| `Hi.` | `h` | no — the word is not starting |
| `hi ` | `h` | no |
| `Really? ` | `y` | yes |
| `Wow! ` | `s` | yes |
| `Wait... ` | `w` | no — an ellipsis |
| `See e.g. ` | `t` | no — an abbreviation |
| `It's at example.com. ` | `t` | yes |
| `Line⏎` | `n` | yes |
| `He said "stop." ` | `t` | yes |
| `(Done.) ` | `n` | yes |
| `"` | `h` | yes |
| `Hi. (` | `s` | yes |
| `- ` | `m` | yes |
| `List:⏎1. ` | `b` | yes |
| `12) ` | `x` | yes |
| `# ` | `t` | yes |
| `> ` | `q` | yes |
| ```` ```⏎ ```` | `l` | no — inside a fence |
| ```` ```⏎code⏎```⏎ ```` | `n` | yes — the fence closed |
| ``Use `x. `` | `y` | no — inside a code span |
| `Hi. ` | `H` | no — already upper case |
| `Hi. ` | `1` | no — not a letter |
| `` (empty) | `ß` | no — its upper case is two letters |
| `Hi.  ` (two spaces) | `h` | yes |
| `Hi.⇥` (a tab) | `h` | yes |
| `` (empty) | `é` | yes |
| `Привет. ` | `к` | yes |
| `Ok.⏎⏎` | `n` | yes |
| `@ann ` | `h` | no |

(`⏎` is a line feed, `⇥` a tab.)

## 2. Formulas, diagrams and graphs

### One renderer

`web/rich/` holds everything that draws a rich block, and every client uses the same files:

| File | What |
|---|---|
| `fc-rich.js` | ours: takes one job, draws it, reports the rectangle (its header is the contract) |
| `rich.html` | ours: the page a native client loads into its hidden web view |
| `katex.min.js`, `katex.min.css`, `mhchem.min.js`, `fonts/*.woff2` | KaTeX 0.17.0 + mhchem — MIT; fonts SIL OFL 1.1 |
| `mermaid.min.js` | Mermaid 11.16.0 — MIT |
| `viz-global.js` | Viz.js 3.24.0 (Graphviz 14.1.1, Expat) — MIT / EPL |
| `plot.js` | md 1.6's plotter (`md.vscode/src/render/plot.ts`), bundled with esbuild — MIT; byte-identical to md's golden SVG |

The engines are md's own copies (byte-identical to md 1.6's `rich/`), minus PlantUML, highlight.js, KaTeX's
auto-render and the woff/ttf fonts (every engine here reads woff2). 5.4 MB.

The copies, each of which must equal `web/rich/` byte for byte (CI's "rich files" step):

- `ios/FamilyConnect/Rich/` (one copy for iOS and macOS)
- `android/app/src/main/assets/rich/`
- `win/src/FamilyConnect.App/Assets/Rich/`

**Change a file in `web/rich/`, copy it to the three, and bump `VERSION` in `fc-rich.js`** — the version is in every
native cache key, so an old drawing is never shown for a new engine.

Measured in headless Chrome with the page's own content policy: a formula 2–17 ms, a Mermaid flowchart ~70 ms the
first time and ~10 ms after, Graphviz ~20 ms first and 2 ms after, a plot ~10 ms. Every failure comes back as an
error rather than a blank.

### The native host

One hidden web view per app, made on first need and kept:

- It loads `rich.html` from the app's copy over a real origin — Apple: a `WKURLSchemeHandler` scheme; Android:
  `WebViewAssetLoader` on `https://appassets.androidplatform.net/rich/`; Windows: a WebView2 virtual host mapped to
  `Assets/Rich` (as `WebViewCallMedia` maps `Assets/Call`). Every other request is refused: no network, ever.
- Transparent background; big enough to hold a drawing (resize before a snapshot if the result is bigger).
- Jobs run one at a time. A job is `fcRich.draw(job)` (Apple can await it with `callAsyncJavaScript`) or
  `fcRich.drawAndPost(id, job)` where the call cannot wait for a promise (Android's `FcRichBridge`, WebView2's
  `postMessage`). Fifteen seconds without an answer is a failure.
- The result rectangle is at the page's top left; snapshot it at the screen's scale, capped so the longest side is at
  most 4096 pixels. The image keeps its alpha.
- If the web content process dies (`webViewWebContentProcessDidTerminate`, `onRenderProcessGone`, WebView2's
  `ProcessFailed`), the job in flight fails and the next one makes a new web view.

The job a client sends:

| Field | Formula | Diagram or graph |
|---|---|---|
| `kind` | `math` (block) or `math-inline` | `mermaid`, `graphviz`, `plot` |
| `engine` | — | Graphviz's layout (`dot` unless the fence named another) |
| `dark` | the theme | the theme |
| `color` | the bubble's text colour (mine and theirs differ) | the card's ink: `#1C1C1E` light, `#F2F2F7` dark |
| `background` | `transparent` | the card: `#FFFFFF` light, `#1C1C1E` dark |
| `fontSize` | the bubble's body text size, in points/dp/px | the same (Mermaid's text size) |

**The cache.** Keyed by SHA-256 of `fcRich.version`, kind, engine, dark, colour, background, font size and source.
A drawing is kept in memory (a small LRU) and on disk in the app's caches folder (PNG, with its width, height and
baseline in CSS pixels). A failure is remembered in memory only, so a newer engine gets another try.

### In the bubble

- A **rich block** is its own block in the bubble, like a table. Drawn: the image, scaled down — never up — to the
  bubble's width, a diagram's card with rounded corners. Tapping a diagram or a graph opens it full size and
  zoomable, using the client's own photo viewer where it has one. Not yet drawn, or failed: the source in the
  monospaced face, exactly as a code block shows it today (a display formula shows its delimiters too).
- An **inline formula** is an image in the run of text, on the baseline (`baseline` in the result, which is CSS pixels
  from the top). Not yet drawn, or failed: its source with its delimiters, monospaced.
- **Accessibility:** a drawn block's label is its source — no new strings to translate.
- When a drawing lands, the bubble grows; the chat keeps its place the way it does when a photo loads.
- Only the three chat bubbles draw anything. Reply excerpts, chat-list previews, pushes, copy and share keep the
  source.

### The parser's shape, per port

Each parser gains a third block, beside text and table, and an inline formula run:

| Port | Block | Inline |
|---|---|---|
| Swift | `Block.rich(RichBlock)` — `kind`, `engine`, `source` | an `AttributedString` attribute carrying the formula's source |
| Kotlin | `Block.Rich(kind, engine, source)` | an annotation on the run (`InlineTextContent` id) |
| C# | `MarkdownRichBlock(RichKind, Engine, Source)` | `MarkdownStyle.Math` (the source) |
| Rust | `Block::Rich(Rich { kind, engine, source })` | `Style::math: Option<String>` |

`kind` is one of `math`, `mermaid`, `graphviz`, `plot` — the job's `kind` is the same word, with `math-inline` for an
inline formula. A rich fence's info string is not otherwise kept: the language of an ordinary code fence is still
dropped.

### The cases every parser's tests hold

Blocks:

| Body | Blocks |
|---|---|
| ```` ```math⏎x^2⏎``` ```` | rich `math` `x^2` |
| ```` ```Mermaid⏎graph TD⏎A-->B⏎``` ```` | rich `mermaid` `graph TD⏎A-->B` (the tag is case-blind) |
| ```` ```mermaid extra words⏎A⏎``` ```` | rich `mermaid` `A` (the first word decides) |
| ```` ```neato⏎graph {a--b}⏎``` ```` | rich `graphviz`, engine `neato` |
| ```` ```gv⏎digraph{a->b}⏎``` ```` | rich `graphviz`, engine `dot` |
| ```` ```plot⏎sin(x)⏎``` ```` | rich `plot` `sin(x)` |
| ```` ```js⏎x⏎``` ```` | a code block, as before |
| ```` ```mermaid⏎A ```` (never closed) | text, exactly as typed |
| `Look:⏎```math⏎x⏎```⏎Done` | text `Look:`, rich `math` `x`, text `Done` |
| `$$⏎x^2⏎$$` | rich `math` `x^2` |
| `$$x^2$$` | rich `math` `x^2` |
| `\[⏎a⏎\]` | rich `math` `a` |
| `  $$ y $$  ` | rich `math` `y` |
| `$$x$$ is it` | text, with inline math `x` then ` is it` |
| `Area $$x$$` | text, with inline math `x` |
| `$$⏎x` (never closed) | text as typed |
| `$$ $$` | text as typed |
| ```` ```⏎$$x$$⏎``` ```` | a code block — nothing inside a fence is math |

Inline:

| Body | Formulas |
|---|---|
| `$x$` | `x` |
| `$5 and $10` | none |
| `costs $5, or $6.` | none |
| `a$b$c` | none — a word character before the opener |
| `$x$s` | none — a word character after the closer |
| `$ x$` | none |
| `$x $` | none |
| `\$x$` | none — the opener is escaped (the text reads `$x$`) |
| `$a\$b$` | `a\$b` — the escaped `$` cannot close |
| `see $$x$$ now` | `x` |
| `\(x^2\)` | `x^2` |
| `` `$x$` `` | none — code |
| `*see $a*b$*` | `a*b`, inside the emphasis |
| `$x⏎y$` | none — a formula does not cross a line |
| `[a](http://x.y/$b$)` | none — a link's destination |
| `| $a$ | b |⏎|---|---|⏎| $c$ | d |` | `a` in the header, `c` in the first row |
| `€$x$` | `x` — `€` is not an ASCII word character |
| `2$x$` | none — `2` is |

## 3. The assistant

The assistant is the likeliest author of all of this, so the server tells it what the bubbles draw:
`FORMATTING_INSTRUCTION` in `server/src/handlers_ai.rs`, one note composed into every system prompt before the
language instruction (which stays last).
