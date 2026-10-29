//! Sentences fc-core and fc-logic look up through a VARIABLE — the report reasons
//! (`family_text::REASONS`) and the join policies (`house_rules::POLICIES`) — said here as
//! literals, so `linux/i18n/generate.py` (which collects `say.get("…")` literals only) puts them in
//! the nine tables (carry-forward "Found in the other clients" 23(b): Windows never translates the
//! four reasons for the same reason). The tests pin each list to the one it stands for, in order.

use fc_core::text::string_catalog::StringCatalog;

/// A report's reason in words — one this client does not know is "Something else"
/// (`family_text::reason_label`'s rule).
pub fn reason_label(reason: &str, say: &dyn StringCatalog) -> String {
    match reason {
        "spam" => say.get("Spam"),
        "harassment" => say.get("Harassment"),
        "inappropriate" => say.get("Inappropriate"),
        _ => say.get("Something else"),
    }
    .to_string()
}

/// The three join policies' words, in `house_rules::POLICIES` order.
pub fn policy_labels(say: &dyn StringCatalog) -> [&str; 3] {
    [
        say.get("Join immediately"),
        say.get("Need approval"),
        say.get("Nobody"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::house_rules::POLICIES;
    use fc_core::text::string_catalog::EnglishCatalog;
    use fc_logic::family_console::family_text::{self, REASONS};

    #[test]
    fn the_literals_are_the_keys_fc_logic_names() {
        let say = EnglishCatalog::new();
        for (code, key) in REASONS {
            assert_eq!(reason_label(code, &say), key);
            assert_eq!(
                reason_label(code, &say),
                family_text::reason_label(code, &say)
            );
        }
        assert_eq!(reason_label("bogus", &say), "Something else");
        let labels = policy_labels(&say);
        for (at, (_, key)) in POLICIES.iter().enumerate() {
            assert_eq!(labels[at], *key);
        }
    }
}
