//! The windows (shell-design §3). S0 owns the main window's frame; P1 adds `gate` and
//! `conversation_window`, P2 `viewer_window`, P4 `board_window` and `settings_window` (W.2) — each
//! adds its own `pub mod` line here at integration.

pub mod board_window;
pub mod conversation_window;
pub mod gate;
pub mod main_window;
pub mod settings_window;
pub mod viewer_window;
