//! Every copy DRAWN of one thing, by its key (`DrawnCopies.cs`) — a voice message the
//! conversation and the thread panel both show is two bubbles, and playing it, its dot going and
//! its "Played" must reach both, not whichever was drawn last. A copy is added as it is drawn (and
//! again whenever it comes back on screen) and removed as it leaves, BY REFERENCE, so a redraw's
//! new copy and the old one it replaces never stand in for each other.
//!
//! The C#'s reference identity is `Rc::ptr_eq`: a copy is an `Rc` of whatever the shell draws.

use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

/// The copies drawn, per key: `K` is what they are of (an attachment's id), `T` one copy as drawn.
#[derive(Debug)]
pub struct DrawnCopies<K, T: ?Sized> {
    rows: HashMap<K, Vec<Rc<T>>>,
    /// The keys in the order first drawn, so [`DrawnCopies::all`] has an order to keep.
    order: Vec<K>,
}

impl<K, T: ?Sized> Default for DrawnCopies<K, T> {
    fn default() -> Self {
        DrawnCopies {
            rows: HashMap::new(),
            order: Vec::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, T: ?Sized> DrawnCopies<K, T> {
    pub fn new() -> Self {
        DrawnCopies::default()
    }

    /// A copy drawn; the same copy twice is still one.
    pub fn add(&mut self, key: K, row: Rc<T>) {
        let copies = self.rows.entry(key.clone()).or_insert_with(|| {
            self.order.push(key);
            Vec::new()
        });
        if !copies.iter().any(|copy| Rc::ptr_eq(copy, &row)) {
            copies.push(row);
        }
    }

    /// A copy gone from the screen; the others of the same key stay. Whether it was there.
    pub fn remove(&mut self, key: &K, row: &Rc<T>) -> bool {
        let Some(copies) = self.rows.get_mut(key) else {
            return false;
        };
        let before = copies.len();
        copies.retain(|copy| !Rc::ptr_eq(copy, row));
        let removed = copies.len() < before;
        if copies.is_empty() {
            self.rows.remove(key);
            self.order.retain(|held| held != key);
        }
        removed
    }

    /// Every copy of one key, in the order drawn; none when it is not on the screen.
    pub fn of(&self, key: &K) -> Vec<Rc<T>> {
        self.rows.get(key).cloned().unwrap_or_default()
    }

    /// Every copy of everything.
    pub fn all(&self) -> Vec<Rc<T>> {
        self.order
            .iter()
            .filter_map(|key| self.rows.get(key))
            .flat_map(|copies| copies.iter().cloned())
            .collect()
    }

    /// Nothing drawn any more: the chat was left.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.order.clear();
    }
}
