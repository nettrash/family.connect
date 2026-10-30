//! `SeveralFamiliesTests.AResetIsOfferedOnlyWhereItWillWork`, whose subject is the console's row
//! (`FamilyText.ToolsFor`) over `FamilyRules.MayResetPassword`: a password reset is offered only
//! where this owner owns every family the member is in — or the roster does not say.

use fc_core::protocol::MemberDto;
use fc_logic::families::family_rules;
use fc_logic::family_console::family_text;

#[test]
fn a_reset_is_offered_only_where_it_will_work() {
    let bob = MemberDto {
        role: Some("member".to_string()),
        ..MemberDto::new(9, "bob", "Bob")
    };
    let says = |reset: Option<bool>| MemberDto {
        password_reset: reset,
        ..bob.clone()
    };
    assert!(family_rules::may_reset_password(&bob, 7, true));
    assert!(family_rules::may_reset_password(&says(Some(true)), 7, true));
    assert!(!family_rules::may_reset_password(
        &says(Some(false)),
        7,
        true
    ));
    assert!(!family_rules::may_reset_password(&bob, 7, false));
    assert!(!family_rules::may_reset_password(&bob, 9, true));
    let tools = family_text::tools_for(&says(Some(false)), 7, true, false);
    assert!(tools.removable);
    assert!(!tools.reset_password);
}
