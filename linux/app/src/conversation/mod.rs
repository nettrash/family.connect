//! The conversation (shell-design §5, P1): one page per open chat ([`page::ConversationPage`]) —
//! the timeline ([`timeline`]) of rows ([`bubble`]) over the composer (`crate::composer`) — with
//! the message menu ([`menu`], [`safety`]), threads ([`thread`]) and polls ([`polls`]). The words
//! of a bubble are Pango markup written by [`markup`] from fc-logic's runs; P1's CSS is [`style`].

pub mod bubble;
pub mod markup;
pub mod menu;
pub mod page;
pub mod polls;
pub mod safety;
pub mod style;
pub mod thread;
pub mod timeline;

pub use page::{ConversationPage, PageHost};
