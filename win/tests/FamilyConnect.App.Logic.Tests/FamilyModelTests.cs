using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The family's own screens: the door, the owner's console, and the numbers everybody may see.
/// </summary>
public class FamilyModelTests : IDisposable
{
    private const long Me = 7;

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;

    public FamilyModelTests()
    {
        chats = new ChatStore(cache, () => Me);
        chats.Replace([
            new MemberDto(7, "anna", "Anna", Role: "owner"),
            new MemberDto(11, "bob", "Bob", Role: "member"),
        ]);
    }

    public void Dispose() => cache.Dispose();

    private (FamilyModel Family, Server Handler) Build(Server server) =>
        (new FamilyModel(
            new ApiClient(
                new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
                new MemoryTokenStore("t0ken")),
            chats),
         server);

    private static FamilyDto Smiths(
        int? cap = null,
        bool vision = false,
        bool photos = false,
        bool faces = false,
        bool history = true,
        string? language = null) =>
        new(
            3, "The Smiths", "open", null, "ABCD2345", cap,
            AiHistory: history, AiVision: vision, AiHistoryPhotos: photos,
            AiGreeting: false, AiFaces: faces, Language: language);

    /// <summary>
    /// THE DOOR TAKES THE LOWER OF TWO NUMBERS, and the family's own setting is not the same
    /// question. A family that set 40 under a ceiling of 50 goes on reporting 40 after the
    /// operator drops the ceiling to 10 — a stored cap is never re-validated when the ceiling
    /// moves, because one config edit must not lock an owner out of their own settings screen.
    /// </summary>
    [Fact]
    public void SeatsLeftIsTheLowerOfTheFamilysCapAndTheOperatorsCeiling()
    {
        var (family, _) = Build(new Server());

        // Its own cap binds.
        var own = family.SeatsFor(Smiths(cap: 4), ceiling: 50);
        Assert.Equal(4, own.Limit);
        Assert.Equal(2, own.Left);
        Assert.False(own.Full);

        // The ceiling binds, and the family's own setting is still 40 — which is what the
        // settings screen shows.
        var ceiling = family.SeatsFor(Smiths(cap: 40), ceiling: 10);
        Assert.Equal(40, ceiling.Cap);
        Assert.Equal(10, ceiling.Limit);
        Assert.Equal(8, ceiling.Left);

        // NO cap of its own is not "a cap equal to the ceiling": it is no cap, and only the
        // operator's ceiling binds.
        var none = family.SeatsFor(Smiths(), ceiling: 3);
        Assert.Null(none.Cap);
        Assert.Equal(3, none.Limit);
        Assert.Equal(1, none.Left);

        // A cap BELOW the room is a freeze rather than a refusal: nobody new until people leave.
        var frozen = family.SeatsFor(Smiths(cap: 1), ceiling: 50);
        Assert.Equal(0, frozen.Left);
        Assert.True(frozen.Full);
    }

    [Fact]
    public void FormerMembersDoNotTakeSeats()
    {
        chats.Replace(
            [new MemberDto(7, "anna", "Anna", Role: "owner")],
            [new MemberDto(11, "bob", "Bob", HasLeft: true, Deleted: true)]);
        var (family, _) = Build(new Server());

        Assert.Equal(1, family.Present().Count);
        Assert.Equal(2, family.Members().Count);
        Assert.Equal(3, family.SeatsFor(Smiths(cap: 4), ceiling: 50).Left);
    }

    /// <summary>
    /// THE ASSISTANT'S TWO DEPENDENT SWITCHES may only be true while <c>ai_vision</c> is, and
    /// turning vision off turns them off in the same write whether or not the request mentioned
    /// them. A screen that drew them as still on would be lying about what the family agreed to.
    /// </summary>
    [Fact]
    public void TurningVisionOffTurnsTheTwoThatDependOnItOff()
    {
        var on = Smiths(vision: true, photos: true, faces: true);

        var off = FamilyModel.AsApplied(on, new FamilyPatch { AiVision = false });

        Assert.False(off.AiVision);
        Assert.False(off.AiHistoryPhotos);
        Assert.False(off.AiFaces);
        // And nothing else moved: absent leaves a field alone.
        Assert.True(off.AiHistory);
        Assert.Equal("ABCD2345", off.InviteCode);

        // Neither is offered while vision is off, because the server would refuse it.
        Assert.False(FamilyModel.MayTurnOn("ai_history_photos", off));
        Assert.False(FamilyModel.MayTurnOn("ai_faces", off));
        // The two that depend on nothing are always offered.
        Assert.True(FamilyModel.MayTurnOn("ai_history", off));
        Assert.True(FamilyModel.MayTurnOn("ai_greeting", off));
    }

    /// <summary>
    /// A dependent switch is never SENT true alongside vision off: that answer is
    /// <c>validation</c>, and the person would be told their own screen was wrong.
    /// </summary>
    [Fact]
    public async Task AChangeTheServerWouldRefuseIsNotSent()
    {
        var (family, handler) = Build(new Server().On("/families/mine", """
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                        "ai_history": true, "ai_vision": false}}
            """));

        await family.ChangeAsync(
            Smiths(),
            new FamilyPatch { AiHistoryPhotos = true, AiFaces = true, AiGreeting = true });

        var sent = Assert.Single(handler.Bodies);
        Assert.DoesNotContain("ai_history_photos", sent);
        Assert.DoesNotContain("ai_faces", sent);
        // What did not depend on vision went.
        Assert.Contains("\"ai_greeting\":true", sent);
    }

    /// <summary>
    /// THE TWO PLACES WHERE `null` MEANS SOMETHING A MISSING KEY DOES NOT: clearing the cap and
    /// clearing the language. Everything else absent means "leave it alone".
    /// </summary>
    [Fact]
    public async Task ClearingTheCapAndTheLanguageSendsNullAndNothingElseSendsAnything()
    {
        var (family, handler) = Build(new Server().On("/families/mine", """
            {"family": {"id": 3, "name": "The Smiths", "ai_history": true}}
            """));

        await family.ChangeAsync(
            Smiths(cap: 8, language: "ru"),
            new FamilyPatch { ClearsCap = true, ClearsLanguage = true });

        var sent = Assert.Single(handler.Bodies);
        Assert.Contains("\"max_members\":null", sent);
        Assert.Contains("\"language\":null", sent);
        Assert.DoesNotContain("join_policy", sent);
        Assert.DoesNotContain("ai_history", sent);

        // And locally the two are cleared rather than left alone.
        var applied = FamilyModel.AsApplied(
            Smiths(cap: 8, language: "ru"),
            new FamilyPatch { ClearsCap = true, ClearsLanguage = true });
        Assert.Null(applied.MaxMembers);
        Assert.Null(applied.Language);
    }

    [Fact]
    public async Task AChangeThatNamesNothingIsAValidNoOp()
    {
        var (family, handler) = Build(new Server().On("/families/mine", """
            {"family": {"id": 3, "name": "The Smiths", "ai_history": true}}
            """));

        var (changed, error) = await family.ChangeAsync(Smiths(), new FamilyPatch());

        Assert.Null(error);
        Assert.Equal("The Smiths", changed!.Name);
        Assert.Equal("{}", Assert.Single(handler.Bodies));
    }

    /// <summary>
    /// A SUCCESSOR IS A PREDICTION. Any join or leave changes it and it takes no frame of its
    /// own, so the dialog is drawn from a FRESH read every time — and its absence on that read
    /// means leaving DELETES the family, which is a different dialog.
    /// </summary>
    [Fact]
    public async Task TheLeaveDialogIsDrawnFromAFreshReadAndNeverFromACache()
    {
        var (family, handler) = Build(new Server().Then("/families/mine",
            (HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                             {"id": 14, "username": "gran", "display_name": "Gran", "role": "member"}],
                 "blocked_user_ids": [], "next_owner_user_id": 14}
                """),
            (HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "blocked_user_ids": []}
                """)));

        var (deletes, successor, error) = await family.LeaveWouldAsync();
        Assert.Null(error);
        Assert.False(deletes);
        // Resolved to a name — from the roster THAT read refreshed, because a member who joined
        // since would not be in the cache.
        Assert.Equal("Gran", successor!.DisplayName);

        // Gran leaves first. The next dialog is a different dialog.
        var (alone, nobody, _) = await family.LeaveWouldAsync();
        Assert.True(alone);
        Assert.Null(nobody);
        Assert.Equal(2, handler.Asked.Count);
    }

    [Fact]
    public async Task LeavingAnswersTheSuccessorTheServerActuallyChose()
    {
        var (family, _) = Build(new Server()
            .On("/families/leave", """{"new_owner_user_id": 11}"""));

        var (successor, error) = await family.LeaveAsync();

        Assert.Null(error);
        Assert.Equal(11, successor);
    }

    /// <summary>
    /// A <c>family_full</c> refusal of an approval LEAVES THE REQUEST PENDING: a full family is a
    /// temporary condition and not a decision, and a client that struck the row off its list
    /// would have thrown the owner's decision away.
    /// </summary>
    [Fact]
    public async Task ApprovingAJoinRequestIntoAFullFamilyLeavesItPending()
    {
        var (family, _) = Build(new Server().On(
            "/families/join-requests/12/approve",
            """{"error": {"code": "family_full", "message": "no"}}""",
            HttpStatusCode.Conflict));

        var (joined, stillPending, error) = await family.ApproveAsync(12);

        Assert.Null(joined);
        Assert.True(stillPending);
        Assert.Equal(ErrorCodes.FamilyFull, error!.Code);
    }

    [Fact]
    public async Task AnApprovedRequestPutsTheMemberOnTheRoster()
    {
        var (family, _) = Build(new Server().On(
            "/families/join-requests/12/approve",
            """
            {"member": {"id": 14, "username": "gran", "display_name": "Gran", "role": "member",
                        "birthday": {"month": 3, "day": 14}}}
            """));

        var (joined, stillPending, error) = await family.ApproveAsync(12);

        Assert.Null(error);
        Assert.False(stillPending);
        Assert.Equal("Gran", joined!.DisplayName);
        Assert.Equal("Gran", chats.Member(14)!.DisplayName);
        // A join carries what the server knows, birthday included — the calendar is drawn from
        // the roster and nothing else replays it.
        Assert.Equal(new BirthdayDto(3, 14), chats.Member(14)!.Birthday);
        // And arriving is not owning.
        Assert.False(chats.Member(14)!.Owner);
    }

    [Fact]
    public async Task RequestsThatNobodyHasMadeAreAnEmptyListAndNotAFailure()
    {
        var (family, _) = Build(new Server().On("/families/join-requests", """{}"""));

        var (requests, error) = await family.RequestsAsync();

        Assert.Null(error);
        Assert.Empty(requests);
    }

    /// <summary>
    /// Blocking is applied HERE as well as asked for: the frame that confirms it reaches this
    /// device too, and full state applied twice is applied once.
    /// </summary>
    [Fact]
    public async Task BlockingTakesEffectWithoutWaitingForTheFrame()
    {
        var (family, handler) = Build(new Server()
            .On("/families/members/11/block", null, HttpStatusCode.NoContent));

        Assert.Null(await family.BlockAsync(11, blocked: true));

        Assert.True(chats.IsBlocked(11));
        Assert.Equal([11L], family.Blocked());
        Assert.Single(handler.Asked);

        Assert.Null(await family.BlockAsync(11, blocked: false));
        Assert.False(chats.IsBlocked(11));
    }

    [Fact]
    public async Task ARefusedBlockChangesNothingHere()
    {
        var (family, _) = Build(new Server().On(
            "/families/members/11/block",
            """{"error": {"code": "not_same_family", "message": "no"}}""",
            HttpStatusCode.Forbidden));

        var refused = await family.BlockAsync(11, blocked: true);

        Assert.Equal("not_same_family", refused!.Code);
        Assert.False(chats.IsBlocked(11));
    }

    /// <summary>
    /// THE ROWS DO NOT ADD UP TO THE TOTALS, AND THE GAP IS THE BLOCK. A client must never derive
    /// a total by summing the rows, nor a share by dividing into that sum — the totals are the
    /// family's numbers, and the rows are what this caller may see of them.
    /// </summary>
    [Fact]
    public async Task TheStatisticsTotalsAreTheFamilysAndTheRowsAreOnlyWhatThisReaderMaySee()
    {
        var (family, _) = Build(new Server().On("/families/mine/stats", """
            {"generated_at": "2026-09-12T12:00:00Z",
             "totals": {"members": 4, "messages": 1284, "board_notes": 7,
                        "attachments": {"count": 96, "photo": 61, "bytes": 734003200,
                                        "stored_bytes": 612368384},
                        "ai": {"questions": 43, "prompt_tokens": 12040,
                               "completion_tokens": 30512, "images": 6}},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 512,
                          "attachments": {"count": 31, "photo": 22, "bytes": 241172480},
                          "ai": {"questions": 12, "images": 2}}]}
            """));

        var (stats, error) = await family.StatsAsync();

        Assert.Null(error);
        Assert.Equal(1284, stats!.Totals.Messages);
        Assert.Equal(4, stats.Totals.Members);
        // One row for four members: the gap is who this reader has blocked.
        Assert.Single(stats.Members!);
        Assert.True(FamilyModel.RowsAreIncomplete(stats));
        Assert.NotEqual(
            stats.Totals.Messages,
            stats.Members!.Sum(member => member.Messages));

        // `bytes` and `stored_bytes` are different numbers and the gap is what dedup saved —
        // and the stored one is a family total, never a per-member share.
        Assert.Equal(734003200, stats.Totals.Attachments!.Bytes);
        Assert.Equal(612368384, stats.Totals.Attachments.StoredBytes);
        Assert.Null(stats.Members![0].Attachments!.StoredBytes);

        // A picture answer is one question, no tokens and one image, so a family reading only
        // the token counts would see the expensive half of the assistant as free.
        Assert.Equal(6, stats.Totals.Ai!.Images);
        Assert.Equal(2, stats.Members![0].Ai!.Images);
    }

    [Fact]
    public async Task StatisticsWithEveryRowVisibleSayNothingIsHidden()
    {
        var (family, _) = Build(new Server().On("/families/mine/stats", """
            {"generated_at": "2026-09-12T12:00:00Z",
             "totals": {"members": 1, "messages": 10, "board_notes": 0},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 10}]}
            """));

        var (stats, _) = await family.StatsAsync();

        Assert.False(FamilyModel.RowsAreIncomplete(stats!));
    }
}
