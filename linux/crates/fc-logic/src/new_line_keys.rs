//! New lines in what people write (issue #95; `NewLineKeys.cs`). Enter on its own sends; Enter
//! held with ANY of the modifiers people reach for — Shift, Ctrl or Alt — is a new line in the
//! field. No setting: everybody's habit works, whichever app taught it. The same answer on every
//! desktop client: the Mac's `NewLineKeys` (⌥ ⌘ ⌃ ⇧) and the web's `composer::return_makes_line`.
//! The Super key is not among them — the desktop keeps its combinations for itself.

use std::borrow::Cow;

/// Enter pressed with these held: a new line, or not (then it is the field's own Enter — Send in
/// a composer). `fc_text::composer::return_makes_line` with no Meta key, which a PC keyboard's
/// desktop keeps.
pub fn enter_makes_line(shift: bool, control: bool, alt: bool) -> bool {
    fc_text::composer::return_makes_line(shift, control, alt, false)
}

/// Lines as every other client writes them. A WinUI `TextBox` kept a line break as a lone
/// carriage return, and sent like that a two-line message reached the web as ONE line (CSS draws a
/// CR as a space, and the web's markdown splits lines at line feeds only) — so a CR, alone or
/// before a LF, goes out as a LF. A GTK buffer holds `\n`, but a pasted or dropped text may carry
/// either, so the rule stays. Mentions carry names, not offsets, so nothing else moves.
///
/// Text without a carriage return comes back as itself, borrowed — the C#'s same string.
pub fn for_wire(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}
