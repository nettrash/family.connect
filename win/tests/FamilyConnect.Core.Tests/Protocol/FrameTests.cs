using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// The realtime frames, read and written as the protocol writes them (docs/protocol.md,
/// "WebSocket protocol"). The JSON in these tests is copied from that document.
/// </summary>
public class FrameTests
{
    [Fact]
    public void AMessageFrameCarriesTheWholeMessage()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
             "body": "Dinner at 7?", "created_at": "2026-09-11T18:00:00Z"}}
            """);
        var message = Assert.IsType<ServerFrame.Message>(frame);
        Assert.Equal(1338, message.Value.Id);
        Assert.Equal(42, message.Value.ChatId);
        Assert.Equal("Dinner at 7?", message.Value.Body);
        Assert.Empty(message.Value.Media);
        // Absent is not empty: a message with no reactions has no field at all, which is how a
        // client tells "no data" from "cleared".
        Assert.Null(message.Value.Reactions);
    }

    [Fact]
    public void AnAckIsThisDevicesOwnSendComingBack()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "ack", "client_msg_id": "8f14e45f-…",
             "message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "8f14e45f-…",
                         "body": "Six works", "created_at": "2026-09-11T18:01:00Z"}}
            """);
        var ack = Assert.IsType<ServerFrame.Ack>(frame);
        Assert.Equal("8f14e45f-…", ack.ClientMsgId);
        Assert.Equal(1338, ack.Value.Id);
    }

    /// <summary>
    /// An edit is a SEPARATE type on purpose: <c>message</c> bumps unread counts and raises a
    /// notification, and an edit must do neither.
    /// </summary>
    [Fact]
    public void AnEditIsNotAMessage()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "message_edited",
             "message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
                         "body": "Dinner at 8?", "created_at": "2026-09-11T18:00:00Z",
                         "edited_at": "2026-09-11T18:05:00Z", "edit_seq": 88}}
            """);
        var edited = Assert.IsType<ServerFrame.MessageEdited>(frame);
        Assert.Equal(88, edited.Value.EditSeq);
        Assert.IsNotType<ServerFrame.Message>(frame);
    }

    [Fact]
    public void ReactionsAreCompleteStateAndNeverADelta()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
             "reactions": [{"user_id": 9, "emoji": "❤️"}]}
            """);
        var reactions = Assert.IsType<ServerFrame.Reactions>(frame);
        Assert.Equal(124, reactions.ReactionSeq);
        Assert.Equal(9, Assert.Single(reactions.Value).UserId);
        // Cleared: the field stays, empty. A parser that dropped the key would leave the old
        // hearts on screen for good.
        var cleared = ServerFrame.Parse(
            """
            {"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 125,
             "reactions": []}
            """);
        Assert.Empty(Assert.IsType<ServerFrame.Reactions>(cleared).Value);
    }

    [Fact]
    public void ABoardNoteArrivesWholeWithItsKindsExtraFields()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "board_note", "note": {"id": 12, "author_id": 7, "kind": "tasks",
             "text": "Saturday", "color": "green", "size": "medium", "font": "plain",
             "x": 0.42, "y": 0.13, "created_at": "…", "updated_at": "…",
             "board_seq": 88, "content_seq": 84,
             "items": [{"id": 11, "text": "Milk", "done": true, "done_by": 9},
                       {"id": 12, "text": "Bread", "done": false}]}}
            """);
        var note = Assert.IsType<ServerFrame.BoardNote>(frame).Note;
        Assert.Equal("tasks", note.Kind);
        Assert.Equal(84, note.ContentSeq);
        Assert.Equal(2, note.TaskList.Count);
        Assert.Equal(9, note.TaskList[0].DoneBy);
        // `done_by` is absent while a line is not done — not zero.
        Assert.Null(note.TaskList[1].DoneBy);
    }

    [Fact]
    public void AnEventNoteCountsWhoIsComingAndKnowsThisReadersAnswer()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "board_note", "note": {"id": 13, "author_id": 7, "kind": "event",
             "text": "Gran's birthday", "color": "blue", "size": "medium", "font": "plain",
             "x": 0.1, "y": 0.2, "board_seq": 90,
             "starts_at": "2026-12-24T17:00:00Z", "place": "Gran's",
             "rsvps": [{"user_id": 7, "answer": "going"}, {"user_id": 9, "answer": "going"},
                       {"user_id": 11, "answer": "maybe"}]}}
            """);
        var note = Assert.IsType<ServerFrame.BoardNote>(frame).Note;
        Assert.Equal(2, note.Count("going"));
        Assert.Equal(1, note.Count("maybe"));
        Assert.Equal(0, note.Count("no"));
        Assert.Equal("maybe", note.MyAnswer(11));
        Assert.Null(note.MyAnswer(99));
        // A list's `[]` is meaningful; an event that is not a list has no items at all.
        Assert.Empty(note.TaskList);
    }

    [Fact]
    public void APhotoNoteCarriesItsPictureAndItsShape()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "board_note", "note": {"id": 14, "author_id": 7, "kind": "photo",
             "text": "", "color": "yellow", "size": "medium", "font": "plain",
             "x": 0.3, "y": 0.3, "board_seq": 91,
             "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734,
                            "width": 600, "height": 1200, "has_preview": true}}}
            """);
        var note = Assert.IsType<ServerFrame.BoardNote>(frame).Note;
        Assert.NotNull(note.Attachment);
        Assert.True(note.Attachment!.IsPhoto);
        Assert.Equal(0.5, note.Attachment.AspectRatio);
        // A caption may be empty on a photo note — that is the bare picture.
        Assert.Equal(string.Empty, note.Text);
    }

    [Fact]
    public void AMessageWithSeveralAttachmentsKeepsTheSendersOrder()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "message", "message": {"id": 1340, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "9d3f1e77-…", "body": "", "created_at": "…",
             "attachments": [{"id": 34, "kind": "photo"}, {"id": 35, "kind": "video"}],
             "attachment": {"id": 34, "kind": "photo"}}}
            """);
        var message = Assert.IsType<ServerFrame.Message>(frame).Value;
        // The plural is what a modern client reads; the singular is the first of it, kept for
        // clients that predate plurality, and it is ignored here.
        Assert.Equal([34L, 35L], message.Media.Select(media => media.Id));
    }

    [Fact]
    public void AnOlderServersSingleAttachmentStillArrives()
    {
        var frame = ServerFrame.Parse(
            """
            {"type": "message", "message": {"id": 1341, "chat_id": 42, "sender_id": 7,
             "client_msg_id": null, "body": "look", "created_at": "…",
             "attachment": {"id": 34, "kind": "photo"}}}
            """);
        var message = Assert.IsType<ServerFrame.Message>(frame).Value;
        Assert.Equal(34, Assert.Single(message.Media).Id);
    }

    [Fact]
    public void TheCallFramesReadAsTheProtocolHasThem()
    {
        var offer = ServerFrame.Parse(
            """
            {"type": "call_offer", "call_id": "6a1f0c3e-…", "chat_id": 42, "from_user_id": 7,
             "sdp": "v=0\r\n…", "video": true}
            """);
        var call = Assert.IsType<ServerFrame.CallOffer>(offer);
        Assert.True(call.Video);
        Assert.Equal(7, call.FromUserId);
        // Absent `video` is a voice call, never false-because-missing.
        var voice = ServerFrame.Parse(
            """
            {"type": "call_offer", "call_id": "x", "chat_id": 42, "from_user_id": 7, "sdp": "v=0"}
            """);
        Assert.False(Assert.IsType<ServerFrame.CallOffer>(voice).Video);

        var ice = ServerFrame.Parse(
            """
            {"type": "call_ice", "call_id": "6a1f0c3e-…",
             "candidate": {"candidate": "candidate:…", "sdp_mid": "0", "sdp_mline_index": 0}}
            """);
        var candidate = Assert.IsType<ServerFrame.CallIce>(ice).Candidate;
        Assert.Equal("0", candidate.SdpMid);
        Assert.Equal(0, candidate.SdpMlineIndex);
        // Each is optional: a stack supplies one, the other, or both.
        var partial = ServerFrame.Parse(
            """
            {"type": "call_ice", "call_id": "x", "candidate": {"candidate": "candidate:…"}}
            """);
        var half = Assert.IsType<ServerFrame.CallIce>(partial).Candidate;
        Assert.Null(half.SdpMid);
        Assert.Null(half.SdpMlineIndex);

        var end = ServerFrame.Parse("""{"type": "call_end", "call_id": "x", "reason": "declined"}""");
        Assert.Equal("declined", Assert.IsType<ServerFrame.CallEnd>(end).Reason);
    }

    [Fact]
    public void AnErrorSaysWhichRequestItAnswersAndNeverBoth()
    {
        var send = ServerFrame.Parse(
            """{"type": "error", "code": "not_chat_member", "message": "…", "client_msg_id": "8f14e45f-…"}""");
        var refused = Assert.IsType<ServerFrame.Error>(send);
        Assert.Equal(ErrorCodes.NotChatMember, refused.Code);
        Assert.Equal("8f14e45f-…", refused.ClientMsgId);
        Assert.Null(refused.CallId);

        var call = ServerFrame.Parse(
            """{"type": "error", "code": "peer_busy", "message": "…", "call_id": "6a1f0c3e-…"}""");
        var busy = Assert.IsType<ServerFrame.Error>(call);
        Assert.Equal("6a1f0c3e-…", busy.CallId);
        Assert.Null(busy.ClientMsgId);
    }

    /// <summary>
    /// A frame this client has never heard of is IGNORED, not an error: a newer server may add a
    /// type, and a client that threw would drop the connection over something it was free to
    /// skip (docs/protocol.md, "Compatibility rules").
    /// </summary>
    [Fact]
    public void AnUnknownFrameIsIgnoredRatherThanFailing()
    {
        Assert.Null(ServerFrame.Parse("""{"type": "fireworks", "colour": "green"}"""));
        Assert.Null(ServerFrame.Parse("not json at all"));
        Assert.Null(ServerFrame.Parse("[]"));
        Assert.Null(ServerFrame.Parse("{}"));
        // A known type whose payload is missing is not half-applied either.
        Assert.Null(ServerFrame.Parse("""{"type": "message"}"""));
        Assert.Null(ServerFrame.Parse("""{"type": "board_note"}"""));
    }

    [Fact]
    public void TheMemberFramesCarryTheFamilyExceptWhenThereIsNone()
    {
        var joined = ServerFrame.Parse(
            """
            {"type": "member_joined", "family_id": 3,
             "user": {"id": 11, "username": "junior", "display_name": "Junior", "avatar_version": 0}}
            """);
        var member = Assert.IsType<ServerFrame.MemberJoined>(joined);
        Assert.Equal(3, member.FamilyId);
        Assert.Equal("Junior", member.User.DisplayName);
        // `family_id` is absent when the deleted account belonged to no family.
        var deleted = ServerFrame.Parse(
            """
            {"type": "member_deleted",
             "member": {"id": 11, "username": "junior", "display_name": "Junior", "deleted": true}}
            """);
        var gone = Assert.IsType<ServerFrame.MemberDeleted>(deleted);
        Assert.Null(gone.FamilyId);
        Assert.True(gone.Member.Deleted);
    }

    [Fact]
    public void TheAssistantsDeltasAndItsFailureBothName()
    {
        var delta = ServerFrame.Parse(
            """{"type": "ai_delta", "chat_id": 42, "message_id": 1339, "text": "Sure"}""");
        Assert.Equal("Sure", Assert.IsType<ServerFrame.AiDelta>(delta).Text);
        var error = ServerFrame.Parse("""{"type": "ai_error", "chat_id": 42, "message_id": 1339}""");
        Assert.Equal(1339, Assert.IsType<ServerFrame.AiError>(error).MessageId);
        Assert.IsType<ServerFrame.Pong>(ServerFrame.Parse("""{"type": "pong"}"""));
    }

    [Fact]
    public void WhatThisClientSendsReadsAsTheProtocolWritesIt()
    {
        var send = ClientFrames.Send(42, "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "Dinner at 7?");
        Assert.Equal(
            """{"type":"send","chat_id":42,"client_msg_id":"8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01","body":"Dinner at 7?"}""",
            send);
        // Optional fields are ABSENT when they do not apply, never null and never empty.
        Assert.DoesNotContain("reply_to_message_id", send, StringComparison.Ordinal);
        Assert.DoesNotContain("attachment_ids", send, StringComparison.Ordinal);
        Assert.DoesNotContain("poll", send, StringComparison.Ordinal);
        Assert.DoesNotContain("mentions", send, StringComparison.Ordinal);

        Assert.Contains(
            "\"reply_to_message_id\":1337",
            ClientFrames.Send(42, "id", "Six works", replyToMessageId: 1337),
            StringComparison.Ordinal);
        Assert.Contains(
            "\"attachment_ids\":[34,35,36]",
            ClientFrames.Send(42, "id", "", attachmentIds: [34, 35, 36]),
            StringComparison.Ordinal);
        Assert.Contains(
            "\"poll\":{\"options\":[\"Pizza\",\"Pasta\"]}",
            ClientFrames.Send(42, "id", "Pizza or pasta?", pollOptions: ["Pizza", "Pasta"]),
            StringComparison.Ordinal);
        Assert.Contains(
            "\"mentions\":[{\"user_id\":9,\"name\":\"Anna\"}]",
            ClientFrames.Send(42, "id", "@Anna are you in?", mentions: [new MentionDto(9, "Anna")]),
            StringComparison.Ordinal);

        Assert.Equal("""{"type":"read","chat_id":42,"last_read_message_id":1337}""",
            ClientFrames.Read(42, 1337));
        Assert.Equal("""{"type":"typing","chat_id":42}""", ClientFrames.Typing(42));
        Assert.Equal("""{"type":"ping"}""", ClientFrames.Ping());
        // A voice call sends no `video` at all; a video call sends true.
        Assert.DoesNotContain("video", ClientFrames.CallOffer("c", 42, "v=0"), StringComparison.Ordinal);
        Assert.Contains("\"video\":true", ClientFrames.CallOffer("c", 42, "v=0", video: true),
            StringComparison.Ordinal);
        // And a candidate sends only the halves it was given.
        var ice = ClientFrames.CallIce("c", new IceCandidate("candidate:…", SdpMlineIndex: 0));
        Assert.Contains("\"sdp_mline_index\":0", ice, StringComparison.Ordinal);
        Assert.DoesNotContain("sdp_mid", ice, StringComparison.Ordinal);
    }

    /// <summary>
    /// Every frame this client sends has to come back through its own parser unchanged — the
    /// cheapest guard against a writer and a reader drifting apart.
    /// </summary>
    [Fact]
    public void WhatIsSentSurvivesItsOwnParser()
    {
        var candidate = new IceCandidate("candidate:…", "0", 0);
        Assert.Equal(
            new ServerFrame.CallIce("c", candidate),
            ServerFrame.Parse(ClientFrames.CallIce("c", candidate)));
        Assert.Equal(
            new ServerFrame.CallEnd("c", "hangup"),
            ServerFrame.Parse(ClientFrames.CallEnd("c", "hangup")));
        Assert.Equal(
            new ServerFrame.CallAnswer("c", "v=0"),
            ServerFrame.Parse(ClientFrames.CallAnswer("c", "v=0")));
    }
}
