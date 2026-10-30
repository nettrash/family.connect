//! `AppVersionTextTests`: the version line, and why the build number is in brackets rather than
//! inline.

use fc_logic::app_version_text::for_version;

/// A stamped build reads the way the Apple apps read — version, then build in brackets.
#[test]
fn a_stamped_build_puts_its_number_in_brackets() {
    assert_eq!(for_version(1, 1, 437), "1.1 (437)");
    assert_eq!(for_version(2, 0, 1), "2.0 (1)");
}

/// Build 0 is what a LOCAL build carries — CI is what stamps a number — and "1.1 (0)" would be a
/// bug report quoting a number that means "nobody stamped this".
#[test]
fn an_unstamped_build_says_only_the_version() {
    assert_eq!(for_version(1, 1, 0), "1.1");
}

/// Invariant digits: a Settings line is not a place for locale-specific numerals. The C# sets
/// ar-EG as the ambient culture; Rust has none, so what is pinned is that the digits are ASCII.
#[test]
fn the_digits_are_invariant() {
    let line = for_version(1, 1, 437);
    assert_eq!(line, "1.1 (437)");
    assert!(line.is_ascii());
}
