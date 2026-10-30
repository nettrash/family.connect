//! Text fields at the shell's edge (shell-design §7, P1): [`offsets`] converts GTK's character
//! offsets and fc-logic's UTF-16 carets both ways (pure, tested); [`typing_hooks`] runs the typing
//! aids — capitals and emoji as you type — on a `gtk::TextView` or a `gtk::Editable`. Spelling is
//! not here: nettrash decided 2026-10-29 that it comes later (D9).

pub mod offsets;
pub mod typing_hooks;
