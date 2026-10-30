//! The family's own screens: the roster, the door, the owner's console and the numbers everybody
//! may see (`Family.cs`).
//!
//! A SUCCESSOR IS A PREDICTION AND NEVER A CACHED VALUE. `next_owner_user_id` changes with any join
//! or leave and takes no frame of its own, so the leave dialog is drawn from a FRESH `GET
//! /families/mine` every time — and its ABSENCE on that fresh read means the owner is the last
//! member and leaving DELETES the family, which is a different dialog and a different
//! confirmation.
//!
//! THE ASSISTANT'S SWITCHES HAVE A DEPENDENCY, AND THIS SIDE KEEPS IT TOO. `ai_history_photos` and
//! `ai_faces` may only be true while `ai_vision` is: the server refuses the combination and turns
//! both off whenever vision goes off, whether or not the request mentioned them. A client that let
//! a person tick a box the server would refuse — or that drew the two as still on after vision went
//! off — would be lying about what the family had agreed to.
//!
//! THE STATISTICS' ROWS DO NOT ADD UP TO THE TOTALS, AND THE GAP IS THE BLOCK. The totals are the
//! family's numbers; the rows are what this caller may see of them. Deriving a total by summing the
//! rows, or a share by dividing into that sum, is how a client tells two members different things
//! about one family.

use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ApiError, FamilyDto, FamilyPatch, JoinRequestDto, MemberDto, StatsResponse, UserDto,
    error_codes,
};
use fc_core::store::{ChatStore, StoreResult};

/// How many people may still join, and why. The two numbers are kept apart on purpose: they can
/// legitimately disagree, and a screen that showed only one of them would be wrong about the
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seats {
    /// The family's OWN cap, or `None` when the owner never set one. Absent is NOT the ceiling: "we
    /// never set one" is not "we set one equal to whatever the ceiling happens to be today", and
    /// the ceiling moves between server restarts.
    pub cap: Option<i32>,
    /// The operator's, from `GET /me`.
    pub ceiling: i32,
    /// How many are in the family now — the owner included.
    pub members: i32,
}

impl Seats {
    /// The door takes the LOWER of the two. A family that set 40 under a ceiling of 50 goes on
    /// reporting 40 after the operator drops the ceiling to 10 — a stored cap is never re-validated
    /// when the ceiling moves — so what is actually left is the lower one.
    pub fn limit(&self) -> i32 {
        self.cap.map_or(self.ceiling, |cap| cap.min(self.ceiling))
    }

    /// How many may still come in. Never negative: a cap below the room is a freeze.
    pub fn left(&self) -> i32 {
        (self.limit() - self.members).max(0)
    }

    /// Whether anybody else can join at all.
    pub fn full(&self) -> bool {
        self.left() == 0
    }
}

/// What the leave dialog must say, read FRESH (`LeaveWouldAsync`'s answer).
#[derive(Debug, Clone, PartialEq)]
pub struct LeaveWould {
    /// Leaving deletes the family: the owner is its last member.
    pub deletes_family: bool,
    /// Who the family would pass to, from the roster that same read refreshed — `None` when it
    /// deletes, or when the successor is not on it.
    pub successor: Option<MemberDto>,
}

/// An approval the server did not take (`ApproveAsync`'s failure).
#[derive(Debug, Clone, PartialEq)]
pub struct NotApproved {
    pub error: ApiError,
    /// `family_full`: the request is still waiting, because a full family is a temporary condition
    /// rather than a decision.
    pub still_pending: bool,
}

/// The family's own screens.
pub struct FamilyModel {
    api: Rc<ApiClient>,
    chats: Rc<ChatStore>,
}

impl std::fmt::Debug for FamilyModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FamilyModel").finish_non_exhaustive()
    }
}

impl FamilyModel {
    pub fn new(api: Rc<ApiClient>, chats: Rc<ChatStore>) -> Self {
        FamilyModel { api, chats }
    }

    /// The roster as this device holds it — including the members who are gone.
    pub fn members(&self) -> StoreResult<Rc<[MemberDto]>> {
        self.chats.members()
    }

    /// Who is still IN the family, which is who the console acts on.
    pub fn present(&self) -> StoreResult<Vec<MemberDto>> {
        Ok(self
            .chats
            .members()?
            .iter()
            .filter(|member| !member.is_former())
            .cloned()
            .collect())
    }

    /// Everyone this reader has chosen not to see, roster or no roster.
    pub fn blocked(&self) -> StoreResult<Rc<[i64]>> {
        self.chats.blocked()
    }

    /// How many seats are left, from the family's own cap and the operator's ceiling — which is the
    /// only honest answer, because either alone can be the binding one.
    pub fn seats_for(&self, family: &FamilyDto, ceiling: i32) -> StoreResult<Seats> {
        Ok(Seats {
            cap: family.max_members,
            ceiling,
            members: i32::try_from(self.present()?.len()).unwrap_or(i32::MAX),
        })
    }

    /// Whether a person may tick this switch as things stand. The two that depend on vision are not
    /// offered while vision is off — the server would refuse them, and an offer that cannot be
    /// taken is a worse answer than a disabled one.
    pub fn may_turn_on(switch_name: &str, family: &FamilyDto) -> bool {
        match switch_name {
            "ai_history_photos" | "ai_faces" => family.ai_vision,
            _ => true,
        }
    }

    /// What the family will look like after this patch, as the SERVER will apply it — so the
    /// screen does not claim the two dependent switches are still on after vision goes off.
    pub fn as_applied(family: &FamilyDto, patch: &FamilyPatch) -> FamilyDto {
        let vision = patch.ai_vision.unwrap_or(family.ai_vision);
        let photos = patch.ai_history_photos.unwrap_or(family.ai_history_photos);
        let faces = patch.ai_faces.unwrap_or(family.ai_faces);
        FamilyDto {
            join_policy: patch
                .join_policy
                .clone()
                .or_else(|| family.join_policy.clone()),
            max_members: if patch.clears_cap {
                None
            } else {
                patch.max_members.or(family.max_members)
            },
            language: if patch.clears_language {
                None
            } else {
                patch.language.clone().or_else(|| family.language.clone())
            },
            ai_history: patch.ai_history.unwrap_or(family.ai_history),
            ai_vision: vision,
            // Turning vision off turns both of these off in the same write, whether or not the
            // request mentioned them.
            ai_history_photos: vision && photos,
            ai_faces: vision && faces,
            ai_greeting: patch.ai_greeting.unwrap_or(family.ai_greeting),
            // Tied to no other switch: turning vision off leaves it exactly where it was
            // (docs/protocol.md, "Transcripts on request").
            ai_transcripts: patch.ai_transcripts.unwrap_or(family.ai_transcripts),
            // Tied to no other switch either (docs/protocol.md, "Looking things up").
            ai_lookups: patch.ai_lookups.unwrap_or(family.ai_lookups),
            // A list that REPLACES the stored one when present; what the server keeps of it is read
            // back from its answer (docs/protocol.md, "Today's weather, for places the owner
            // chose").
            greeting_places: patch
                .greeting_places
                .clone()
                .or_else(|| family.greeting_places.clone()),
            ..family.clone()
        }
    }

    /// Send a change, having first made it one the server can accept: a dependent switch is never
    /// sent true alongside vision off, because that answer is `validation` and the person would be
    /// told their own screen was wrong.
    pub async fn change(
        &self,
        family: &FamilyDto,
        patch: &FamilyPatch,
    ) -> Result<FamilyDto, ApiError> {
        let vision = patch.ai_vision.unwrap_or(family.ai_vision);
        let sending = if vision {
            patch.clone()
        } else {
            FamilyPatch {
                ai_history_photos: None,
                ai_faces: None,
                ..patch.clone()
            }
        };
        let answer = self.api.patch_family(&sending).await.into_result()?;
        Ok(answer.family)
    }

    /// What the leave dialog must say, read FRESH. Answers the successor where there is one; no
    /// successor on that read means leaving deletes the family.
    pub async fn leave_would(&self) -> StoreResult<Result<LeaveWould, ApiError>> {
        let family = match self.api.family(None).await.into_result() {
            Ok(family) => family,
            Err(error) => return Ok(Err(error)),
        };
        // The roster is refreshed by the same read, because the successor has to resolve to a name
        // and a member who joined since would not be in the cache.
        self.chats.replace_members(
            family.members.as_deref().unwrap_or(&[]),
            family.former_members.as_deref().unwrap_or(&[]),
        )?;
        let Some(successor) = family.next_owner_user_id else {
            return Ok(Ok(LeaveWould {
                deletes_family: true,
                successor: None,
            }));
        };
        Ok(Ok(LeaveWould {
            deletes_family: false,
            successor: self.chats.member(successor)?,
        }))
    }

    /// The roster read again from the family's own document — after an owner's change to somebody
    /// (a removal, a birthday, an approval), so the screen draws what the server now holds rather
    /// than what this device asked for.
    pub async fn read(&self) -> StoreResult<Option<ApiError>> {
        let family = match self.api.family(None).await.into_result() {
            Ok(family) => family,
            Err(error) => return Ok(Some(error)),
        };
        self.chats.replace_members(
            family.members.as_deref().unwrap_or(&[]),
            family.former_members.as_deref().unwrap_or(&[]),
        )?;
        // Applied only when it is THERE: an absent list is an older server, not an empty one.
        if let Some(blocked) = &family.blocked_user_ids {
            self.chats.replace_blocked(blocked)?;
        }
        Ok(None)
    }

    /// Leave. The answer names the successor the server actually chose (`None`: nobody, which is
    /// a member's leave or the family going with its last member).
    ///
    /// `family_id`: the family to leave — the one shown — or `None` for the home family of a server
    /// that knows one.
    pub async fn leave(&self, family_id: Option<i64>) -> Result<Option<i64>, ApiError> {
        let left = self.api.leave_family(family_id).await.into_result()?;
        Ok(left.new_owner_user_id)
    }

    /// The requests waiting for an answer, oldest first.
    pub async fn requests(&self) -> Result<Vec<JoinRequestDto>, ApiError> {
        let answer = self.api.join_requests().await.into_result()?;
        Ok(answer.requests.unwrap_or_default())
    }

    /// Approve one. A `family_full` refusal leaves the request PENDING and says so, because a full
    /// family is a temporary condition rather than a decision — the owner may come back to it once
    /// a seat frees, and a client that struck the row off its list would have thrown the decision
    /// away.
    pub async fn approve(&self, request_id: i64) -> StoreResult<Result<MemberDto, NotApproved>> {
        match self
            .api
            .approve_join_request(request_id)
            .await
            .into_result()
        {
            Ok(answer) => {
                let member = answer.member;
                // With the answer's `password_reset`: the owner is not offered a reset for a
                // newcomer who is also in a family they do not own, which the server would refuse
                // (docs/protocol.md, "Several families").
                let user = UserDto {
                    avatar_version: member.avatar_version,
                    birthday: member.birthday,
                    ..UserDto::new(member.id, &member.username, &member.display_name)
                };
                self.chats.joined(&user, None, member.password_reset)?;
                Ok(Ok(member))
            }
            Err(error) => Ok(Err(NotApproved {
                still_pending: error.code == error_codes::FAMILY_FULL,
                error,
            })),
        }
    }

    /// Refuse one. It leaves the list either way.
    pub async fn reject(&self, request_id: i64) -> Option<ApiError> {
        self.api
            .reject_join_request(request_id)
            .await
            .error()
            .cloned()
    }

    /// Stop seeing a member, and apply it here at once: the frame that confirms it reaches this
    /// device too, and applying full state twice is applying it once.
    pub async fn block(&self, user_id: i64, blocked: bool) -> StoreResult<Option<ApiError>> {
        let answer = if blocked {
            self.api.block(user_id).await
        } else {
            self.api.unblock(user_id).await
        };
        if let Err(error) = answer.into_result() {
            return Ok(Some(error));
        }
        self.chats.set_blocked(user_id, blocked)?;
        Ok(None)
    }

    /// The numbers, exactly as the server answered them. The rows are NOT summed into the totals
    /// and never will be: the gap between them is this reader's own block list.
    pub async fn stats(&self) -> Result<StatsResponse, ApiError> {
        self.api.stats().await.into_result()
    }

    /// How much of the family's messages this reader can see rows for — the honest way to say "and
    /// some members are hidden", rather than pretending the rows are the whole.
    pub fn rows_are_incomplete(stats: &StatsResponse) -> bool {
        let rows = stats.members.as_ref().map_or(0, Vec::len);
        i64::try_from(rows).unwrap_or(i64::MAX) < i64::from(stats.totals.members)
    }
}
