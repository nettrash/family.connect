//! `LiveMarksTests`, the peer's half: what the live frames leave on screen and nowhere else — the
//! other person's read marker.

use std::cell::RefCell;
use std::rc::Rc;

use fc_logic::peer_reads::PeerReads;

#[test]
fn a_peers_marker_only_goes_forward_and_is_per_chat() {
    let reads = PeerReads::new();
    let changed = Rc::new(RefCell::new(Vec::new()));
    let heard = changed.clone();
    reads
        .changed
        .subscribe(move |chat_id| heard.borrow_mut().push(*chat_id));

    assert_eq!(reads.up_to(42), 0);
    assert!(reads.apply(42, 10));
    assert!(!reads.apply(42, 9));
    assert!(!reads.apply(42, 10));
    assert!(reads.apply(43, 3));
    assert_eq!((reads.up_to(42), reads.up_to(43)), (10, 3));
    assert_eq!(*changed.borrow(), vec![42, 43]);

    reads.clear();
    assert_eq!(reads.up_to(42), 0);
}
