using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// TRANSIENT OR TERMINAL — every failure is one or the other, and the whole send pipeline hangs
/// on telling them apart (docs/protocol.md, "Error shape").
/// </summary>
public class ApiErrorTests
{
    [Fact]
    public void ATransientFailureIsNotARefusal()
    {
        // Nothing was read, so nothing was refused.
        Assert.True(ApiError.Transport("connection reset").Transient);
        Assert.True(new ApiError(ErrorCodes.Internal, "…", 500).Transient);
        Assert.True(new ApiError(ErrorCodes.TooManyRequests, "…", 429).Transient);
        Assert.True(new ApiError(ErrorCodes.Validation, "…", 408).Transient);
        Assert.True(new ApiError("", "…", 502).Transient);
        Assert.True(new ApiError("", "…", 503).Transient);
        // nginx answers its own rate limit with an HTML body, so a 429 may arrive with no code at
        // all — status alone has to be enough to classify it.
        Assert.True(new ApiError("", "rate limited", 429).Transient);
    }

    [Fact]
    public void ATerminalFailureIsTheServerHavingReadTheRequestAndRefusedIt()
    {
        Assert.False(new ApiError(ErrorCodes.MessageTooLong, "…", 400).Transient);
        Assert.False(new ApiError(ErrorCodes.NotChatMember, "…", 403).Transient);
        Assert.False(new ApiError(ErrorCodes.NoteNotFound, "…", 404).Transient);
        Assert.False(new ApiError(ErrorCodes.BoardFull, "…", 409).Transient);
        Assert.False(new ApiError(ErrorCodes.InvalidTask, "…", 422).Transient);
    }

    /// <summary>
    /// <c>invalid_credentials</c> shares the 401 and not the meaning. A client that signs somebody
    /// out for a mistyped current password has read the status and not the answer.
    /// </summary>
    [Fact]
    public void AWrongPasswordIsNotAnExpiredSession()
    {
        Assert.True(new ApiError(ErrorCodes.Unauthorized, "…", 401).SessionGone);
        Assert.False(new ApiError(ErrorCodes.InvalidCredentials, "…", 401).SessionGone);
        Assert.False(new ApiError(ErrorCodes.Validation, "…", 400).SessionGone);
        // And neither is a transport failure, whatever it looks like to the user.
        Assert.False(ApiError.Transport("offline").SessionGone);
    }

    /// <summary>
    /// A refusal can arrive with NO STATUS AT ALL: a socket <c>error</c> frame carries a code and
    /// nothing else. Reading the missing status as "it never got there" would retry a message the
    /// server has already refused, and show the failure six delays late.
    /// </summary>
    [Fact]
    public void ASocketRefusalHasNoStatusAndIsStillARefusal()
    {
        Assert.False(new ApiError(ErrorCodes.MessageTooLong, "…").Transient);
        Assert.False(new ApiError(ErrorCodes.NotChatMember, "…").Transient);
        Assert.False(new ApiError(ErrorCodes.PeerBusy, "…").Transient);
        // Except the two codes that ARE transient wherever they come from.
        Assert.True(new ApiError(ErrorCodes.Internal, "…").Transient);
        Assert.True(new ApiError(ErrorCodes.TooManyRequests, "…").Transient);
        // And a code nobody knows, with no status, is the transport failure it looks like.
        Assert.True(new ApiError("", "…").Transient);
        Assert.True(new ApiError("gremlins", "…").Transient);
        // A NEWER server's code, arriving with a 4xx, is terminal — the status decides when the
        // code is not one we know.
        Assert.False(new ApiError("family_hibernating", "…", 409).Transient);
        Assert.True(new ApiError("family_hibernating", "…", 503).Transient);
    }

    [Fact]
    public void TheErrorBodyDecodesAsTheProtocolWritesIt()
    {
        var envelope = Wire.Decode<ErrorEnvelope>(
            """{"error": {"code": "username_taken", "message": "username is already in use"}}""");
        Assert.NotNull(envelope);
        Assert.Equal(ErrorCodes.UsernameTaken, envelope!.Error.Code);
        Assert.Equal("username is already in use", envelope.Error.Message);
        // A body that is not the shape at all does not throw — the status still classifies it.
        Assert.Null(Wire.Decode<ErrorEnvelope>("<html>429 Too Many Requests</html>"));
    }

    /// <summary>
    /// The canonical list is kept WHOLE, retired codes included: a code that vanishes from the
    /// document is a code somebody deletes from a client that is still talking to an old server.
    /// </summary>
    [Fact]
    public void TheCanonicalCodesAreAllHereAndAllDistinct()
    {
        Assert.Equal(71, ErrorCodes.All.Length);
        // The sticker pack's four, as the document lists them.
        Assert.Contains(ErrorCodes.PackFull, ErrorCodes.All);
        Assert.Contains(ErrorCodes.PackItemTooLarge, ErrorCodes.All);
        Assert.Contains(ErrorCodes.PackItemNotFound, ErrorCodes.All);
        Assert.Contains(ErrorCodes.NotPackItemAuthor, ErrorCodes.All);
        Assert.Equal(ErrorCodes.All.Length, ErrorCodes.All.Distinct().Count());
        Assert.Contains(ErrorCodes.OwnerCannotLeave, ErrorCodes.All);
        Assert.Contains(ErrorCodes.PicturesUnavailable, ErrorCodes.All);
        Assert.Contains(ErrorCodes.PictureRefused, ErrorCodes.All);
        Assert.Contains(ErrorCodes.InvalidTask, ErrorCodes.All);
        // Missing from the document's list until 2026-10-02, and listed there now — so listed here too. Terminal with
        // or without its status: a socket frame carrying it has none, and it is still a refusal.
        Assert.Contains(ErrorCodes.AssistantConsentRequired, ErrorCodes.All);
        Assert.False(new ApiError(ErrorCodes.AssistantConsentRequired, "x", 403).Transient);
        Assert.False(new ApiError(ErrorCodes.AssistantConsentRequired, "x").Transient);
        // The transcript endpoint's four (docs/protocol.md, "Transcripts on request"), all refusals.
        foreach (var code in new[]
                 {
                     ErrorCodes.TranscriptsUnavailable, ErrorCodes.TranscriptNotAllowed, ErrorCodes.NotTranscribable,
                     ErrorCodes.TranscriptRefused,
                 })
        {
            Assert.Contains(code, ErrorCodes.All);
            Assert.False(new ApiError(code, "x", 400).Transient);
        }
        // This client's own name for "it never got there" is NOT one of the protocol's.
        Assert.DoesNotContain(ErrorCodes.Transport, ErrorCodes.All);
        Assert.All(ErrorCodes.All, code => Assert.DoesNotContain("_", code[..1]));
    }
}
