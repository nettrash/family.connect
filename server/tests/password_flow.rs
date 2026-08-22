//! Integration: changing your own password, and the owner resetting a
//! member's (protocol.md, "Auth"). The interesting half is not the hash —
//! it is which SESSIONS survive, because that is what makes a reset a
//! recovery rather than a formality.

mod common;

use common::{assert_error, spawn_server};
use serde_json::json;

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn changing_your_own_password_needs_the_current_one() {
    let server = spawn_server().await;
    let (token, _) = server.register("olive", "Olive").await;

    // A live session is not proof: the case this protects against is an
    // unattended unlocked phone, where the attacker HAS the session.
    assert_error(
        server
            .post(
                &token,
                "/me/password",
                json!({"current_password": "wrong-password", "new_password": "brand-new-one"}),
            )
            .await,
        401,
        "invalid_credentials",
    )
    .await;

    // And the new one still has to be a password.
    assert_error(
        server
            .post(
                &token,
                "/me/password",
                json!({"current_password": "password123", "new_password": "short"}),
            )
            .await,
        400,
        "validation",
    )
    .await;

    assert_eq!(
        server
            .post(
                &token,
                "/me/password",
                json!({"current_password": "password123", "new_password": "brand-new-one"}),
            )
            .await
            .status(),
        204
    );

    // The old password is gone and the new one works.
    assert_error(
        server.login_raw("olive", "password123").await,
        401,
        "invalid_credentials",
    )
    .await;
    assert_eq!(
        server.login_raw("olive", "brand-new-one").await.status(),
        200
    );
}

/// A change ends every OTHER session — that is the recovery — while the
/// device doing the change stays signed in.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_change_revokes_the_other_sessions_but_not_this_one() {
    let server = spawn_server().await;
    let (first, _) = server.register("olive", "Olive").await;
    let second = server.login("olive", "password123").await;

    assert_eq!(server.get(&second, "/me").await.status(), 200);

    assert_eq!(
        server
            .post(
                &first,
                "/me/password",
                json!({"current_password": "password123", "new_password": "brand-new-one"}),
            )
            .await
            .status(),
        204
    );

    // The other device finds out the ordinary way.
    assert_eq!(server.get(&second, "/me").await.status(), 401);
    // The one that made the change is still signed in — otherwise every
    // password change would log you out of the phone in your hand.
    assert_eq!(server.get(&first, "/me").await.status(), 200);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_owner_resets_a_forgotten_password() {
    let server = spawn_server().await;
    let (owner, _) = server.register("olive", "Olive").await;
    let (member, _) = server.register("junior", "Junior").await;
    let (_, invite_code) = server.create_family(&owner, "The Smiths").await;
    server.set_open_policy(&owner).await;
    server.join(&member, &invite_code, "joined").await;
    let member_id = server.user_id(&member).await;

    // A second device of the member's, to prove ALL of them go.
    let member_other = server.login("junior", "password123").await;
    assert_eq!(server.get(&member_other, "/me").await.status(), 200);

    assert_eq!(
        server
            .post(
                &owner,
                &format!("/families/members/{member_id}/password"),
                json!({"new_password": "fresh-start-42"}),
            )
            .await
            .status(),
        204
    );

    // Every session the member had is gone — including the one they were
    // holding, which is the point when an account is compromised.
    assert_eq!(server.get(&member, "/me").await.status(), 401);
    assert_eq!(server.get(&member_other, "/me").await.status(), 401);
    // The owner is untouched.
    assert_eq!(server.get(&owner, "/me").await.status(), 200);

    assert_error(
        server.login_raw("junior", "password123").await,
        401,
        "invalid_credentials",
    )
    .await;
    assert_eq!(
        server.login_raw("junior", "fresh-start-42").await.status(),
        200
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_owner_resets_and_only_inside_the_family() {
    let server = spawn_server().await;
    let (owner, _) = server.register("olive", "Olive").await;
    let (member, _) = server.register("junior", "Junior").await;
    let (stranger, _) = server.register("stranger", "Sam").await;
    let (_, invite_code) = server.create_family(&owner, "The Smiths").await;
    server.set_open_policy(&owner).await;
    server.join(&member, &invite_code, "joined").await;
    let owner_id = server.user_id(&owner).await;
    let member_id = server.user_id(&member).await;
    let stranger_id = server.user_id(&stranger).await;

    // A member cannot reset anyone, including the owner.
    assert_error(
        server
            .post(
                &member,
                &format!("/families/members/{owner_id}/password"),
                json!({"new_password": "fresh-start-42"}),
            )
            .await,
        403,
        "not_family_owner",
    )
    .await;

    // Somebody outside the family answers the same whether they exist or
    // not — the endpoint never confirms ids.
    assert_error(
        server
            .post(
                &owner,
                &format!("/families/members/{stranger_id}/password"),
                json!({"new_password": "fresh-start-42"}),
            )
            .await,
        403,
        "not_same_family",
    )
    .await;
    assert_error(
        server
            .post(
                &owner,
                "/families/members/999999/password",
                json!({"new_password": "fresh-start-42"}),
            )
            .await,
        403,
        "not_same_family",
    )
    .await;

    // And the owner cannot use it on themselves to skip the current-password
    // check that /me/password makes.
    assert_error(
        server
            .post(
                &owner,
                &format!("/families/members/{owner_id}/password"),
                json!({"new_password": "fresh-start-42"}),
            )
            .await,
        400,
        "validation",
    )
    .await;

    // The member's password never changed through any of that.
    assert_eq!(
        server.login_raw("junior", "password123").await.status(),
        200
    );
    let _ = member_id;
}
