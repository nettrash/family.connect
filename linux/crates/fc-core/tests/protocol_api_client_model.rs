//! The one test of `win/tests/FamilyConnect.Core.Tests/Protocol/ApiClientTests.cs` that asks the
//! model alone and not the client: the video message's two limits. The rest of that file drives
//! `ApiClient` and is ported with it.

use fc_core::protocol::{FamilyDto, FamilyResponse, RoundVideoLimits};

/// Both keys, or nothing: one alone — or a number no server sends — is a server this client
/// cannot record for, and a video entry that ends in a refusal is worse than none.
#[test]
fn the_limits_are_both_or_nothing() {
    for (ms, bytes) in [
        (Some(60_000), None),
        (None, Some(12_582_912)),
        (Some(0), Some(12_582_912)),
        (Some(60_000), Some(0)),
        (Some(-1), Some(12_582_912)),
        (Some(60_000), Some(-5)),
    ] {
        let family = FamilyResponse {
            max_round_video_ms: ms,
            max_round_video_bytes: bytes,
            ..FamilyResponse::new(FamilyDto::new(3, "The Smiths"))
        };
        assert_eq!(RoundVideoLimits::of(&family), None, "{ms:?} {bytes:?}");
    }
}
