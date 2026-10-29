//! The form behind "Poll" (`PollDraft.cs`): a question and between two and ten options. It opens
//! with two empty rows, because a poll with one option is not a poll and an empty list looks
//! broken.
//!
//! Create stays off until the draft is a poll the server would take — the apps' way, which says
//! nothing it would have to translate — and [`PollDraft::checked`] answers the CLEANED question and
//! options, so what was checked is exactly what leaves. The check is `fc_core::text::polls`
//! (the server's own `str::trim`, `chars()` and `to_lowercase`).

use fc_core::text::polls;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollDraft {
    pub question: String,
    options: Vec<String>,
}

impl Default for PollDraft {
    fn default() -> Self {
        PollDraft {
            question: String::new(),
            options: vec![String::new(), String::new()],
        }
    }
}

impl PollDraft {
    pub fn new() -> Self {
        PollDraft::default()
    }

    pub fn options(&self) -> &[String] {
        &self.options
    }

    pub fn may_add(&self) -> bool {
        self.options.len() < polls::MAX_OPTIONS
    }

    pub fn may_remove(&self) -> bool {
        self.options.len() > polls::MIN_OPTIONS
    }

    /// An option's words; an index that is not a row changes nothing. (Signed, as the C#'s: a row
    /// a list view reports as −1 is no row.)
    pub fn set(&mut self, index: isize, text: &str) {
        if let Some(option) = usize::try_from(index)
            .ok()
            .and_then(|index| self.options.get_mut(index))
        {
            *option = text.to_string();
        }
    }

    pub fn add(&mut self) -> bool {
        if !self.may_add() {
            return false;
        }
        self.options.push(String::new());
        true
    }

    pub fn remove(&mut self, index: isize) -> bool {
        match usize::try_from(index) {
            Ok(index) if self.may_remove() && index < self.options.len() => {
                self.options.remove(index);
                true
            }
            _ => false,
        }
    }

    /// The question and options to send, or `None` while the draft is not yet a poll.
    pub fn checked(&self) -> Option<(String, Vec<String>)> {
        if !polls::has_question(&self.question) {
            return None;
        }
        let sent = polls::sanitized(&self.options)?;
        Some((polls::trimmed(&self.question).to_string(), sent))
    }
}
