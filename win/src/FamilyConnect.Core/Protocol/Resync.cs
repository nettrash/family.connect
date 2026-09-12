using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// What a client does on every (re)connect (docs/protocol.md, "Best-effort delivery").
/// </summary>
/// <remarks>
/// <para>
/// The socket is a live wire, not a queue — a slow client's socket is dropped and REST is the
/// source of truth — so the order below is the protocol's own, and two of its rules are the kind
/// that only look small:
/// </para>
/// <para>
/// <b>THE FLUSH IS NOT A STEP.</b> It is written fourth because it reads well fourth. The outbox
/// is an obligation ordered against nothing: a client that could not finish — or could not START —
/// the reads must flush anyway. Both of this app's own ports first implemented the list literally,
/// with the flush as the tail of a function that returned early whenever a read failed, which put
/// the one operation that recovers a stuck message behind the reads most likely to fail on the
/// network that stuck it.
/// </para>
/// <para>
/// <b>THE MESSAGE CURSOR BELONGS TO THE LOOP.</b> It is read ONCE, before the first page, and then
/// advanced by the largest id each page actually returned. Re-reading <c>max(id)</c> from the store
/// between pages lets a live message arriving mid-loop jump the cursor to its id, and every message
/// between the last page and it is skipped — permanently, because <c>after_id</c> can never look
/// back and history paging only ever goes older than the oldest row held.
/// </para>
/// </remarks>
public sealed class Resync(
    ApiClient api,
    ChatStore chats,
    BoardStore board,
    SendPipeline? sending = null)
{
    /// <summary>How many rows a catch-up page asks for. The server's own default.</summary>
    public const int PageSize = 50;

    /// <summary>What one pass did, for a log and for a test.</summary>
    public sealed record Report(
        bool Flushed = false,
        bool Signed = false,
        bool HasFamily = false,
        int Chats = 0,
        int Messages = 0,
        int Reactions = 0,
        int Edits = 0,
        int Polls = 0,
        int Notes = 0,
        ApiError? Stopped = null)
    {
        /// <summary>Whether every read finished. A flush that ran anyway is not a failure.</summary>
        public bool Complete => Stopped is null;
    }

    /// <summary>
    /// One pass. Answers what it managed; a read that fails stops the READS and nothing else.
    /// </summary>
    public async Task<Report> RunAsync(CancellationToken ct = default)
    {
        var report = new Report();

        // FIRST, and unconditionally: it costs one request per queued message, it is idempotent,
        // and an early flush is never wrong.
        if (sending is not null)
        {
            await sending.FlushAsync(SendRules.FlushTrigger.SocketConnected, ct).ConfigureAwait(false);
            report = report with { Flushed = true };
        }

        // 1. Who I am, and what this server can do.
        var me = await api.Me(ct).ConfigureAwait(false);
        if (!me.Ok || me.Value is null)
        {
            return report with { Stopped = me.Error };
        }
        report = report with { Signed = true, HasFamily = me.Value.Family is not null };
        // The block list is COMPLETE STATE and the one read where an absent list means nobody
        // rather than "leave what you hold alone" — there is no catch-up feed for it and nothing
        // to miss, which is exactly why it is re-read in full on every pass.
        chats.ReplaceBlocked(me.Value.BlockedUserIds ?? []);

        // The roster and the board's high-water mark live only on `/families/mine`, and nothing
        // else replays a roster change missed while offline — a join, a leave, a birthday, the
        // join policy, the cap and the family's language raise no frame at all, or one a sleeping
        // client did not get.
        long? boardMark = null;
        if (me.Value.Family is not null)
        {
            var family = await api.Family(ct).ConfigureAwait(false);
            if (!family.Ok || family.Value is null)
            {
                return report with { Stopped = family.Error };
            }
            chats.Replace(family.Value.Members ?? [], family.Value.FormerMembers);
            // The same list again, from the family's own document, and applied only when it is
            // THERE. `/me` is the read that always carries it, so an absence here is an older
            // server rather than an empty list — the one place on this pass where absent means
            // "leave alone", because the fact has already arrived in full from the read whose
            // own document it is.
            if (family.Value.BlockedUserIds is { } blocked)
            {
                chats.ReplaceBlocked(blocked);
            }
            boardMark = family.Value.MaxBoardSeq;
        }

        // 2. The list: previews, the authoritative unread counts, and the caller's own marker.
        var list = await api.Chats(ct).ConfigureAwait(false);
        if (!list.Ok || list.Value is null)
        {
            return report with { Stopped = list.Error };
        }
        var rows = list.Value.Chats ?? [];
        chats.Replace(rows);
        report = report with { Chats = rows.Length };

        // 3. Per chat: the messages, then the three sequence catch-ups.
        foreach (var row in rows)
        {
            var caught = await CatchUpAsync(row, ct).ConfigureAwait(false);
            report = report with
            {
                Messages = report.Messages + caught.Messages,
                Reactions = report.Reactions + caught.Reactions,
                Edits = report.Edits + caught.Edits,
                Polls = report.Polls + caught.Polls,
                Stopped = caught.Stopped,
            };
            if (caught.Stopped is not null)
            {
                return report;
            }
        }

        // The wall, if there is a family to have one.
        if (me.Value.Family is not null)
        {
            var notes = await BoardAsync(boardMark, ct).ConfigureAwait(false);
            report = report with { Notes = notes.Notes, Stopped = notes.Stopped };
        }
        return report;
    }

    /// <summary>One chat's four loops: the messages, then the three sequence feeds.</summary>
    private async Task<Report> CatchUpAsync(ChatRowDto row, CancellationToken ct)
    {
        var report = new Report();
        var chatId = row.Chat.Id;

        // THE CURSOR IS READ ONCE. From here on it is advanced by what the pages return, and
        // never re-read from the store.
        var cursor = chats.Messages(chatId, limit: 1).FirstOrDefault()?.Id ?? 0;
        while (true)
        {
            var page = await api.MessagesAfter(chatId, cursor, PageSize, ct).ConfigureAwait(false);
            if (!page.Ok || page.Value is null)
            {
                return report with { Stopped = page.Error };
            }
            var messages = page.Value.Messages ?? [];
            if (messages.Length == 0)
            {
                break;
            }
            chats.Apply(messages);
            report = report with { Messages = report.Messages + messages.Length };
            // A CURSOR THAT CANNOT MOVE ENDS THE LOOP. Every one of these four feeds answers
            // strictly newer rows, so a full page always advances one — but "always" here is the
            // server's promise, not this client's arithmetic, and a full page that did not move
            // the cursor would be asked for again with the same argument until the process died.
            var next = messages.Max(message => message.Id);
            if (next <= cursor)
            {
                break;
            }
            cursor = next;
            if (messages.Length < PageSize)
            {
                // A short page is the end of the loop.
                break;
            }
        }

        // Then the reaction catch-up, and only when the chat says there is something to catch up
        // to: `max_reaction_seq` from step 2 against the cursor this device holds.
        var reactionCursor = chats.Chat(chatId)?.MaxReactionSeq ?? 0;
        if ((row.MaxReactionSeq ?? 0) > reactionCursor)
        {
            var seq = reactionCursor;
            while (true)
            {
                var page = await api.ReactionsAfter(chatId, seq, PageSize, ct).ConfigureAwait(false);
                if (!page.Ok || page.Value is null)
                {
                    return report with { Stopped = page.Error };
                }
                var states = page.Value.MessageReactions ?? [];
                if (states.Length == 0)
                {
                    break;
                }
                foreach (var state in states)
                {
                    // A state for a message this device does not hold is DROPPED — history paging
                    // re-delivers it embedded on the message itself — and the cursor advances all
                    // the same, which is why the ROUTE is what says so: the store moves the chat
                    // on for a page and a frame, and never for evidence.
                    chats.ApplyReactions(
                        chatId, state.MessageId, state.ReactionSeq, state.Reactions,
                        SeqRoute.CatchUpPage);
                }
                report = report with { Reactions = report.Reactions + states.Length };
                var next = states.Max(state => state.ReactionSeq);
                if (next <= seq)
                {
                    break;
                }
                seq = next;
                if (states.Length < PageSize)
                {
                    break;
                }
            }
        }

        // Then the edits. This is the ONLY step that learns of a change to a message this device
        // already holds: `after_id` is `WHERE id > cursor` and cannot look at an older row, so
        // without it a device that slept through an edit shows the old words until something else
        // happens to that message. The feed answers whole messages, so they go through the very
        // path a page of history goes through — guard included.
        var editCursor = chats.Chat(chatId)?.MaxEditSeq ?? 0;
        if ((row.MaxEditSeq ?? 0) > editCursor)
        {
            var seq = editCursor;
            while (true)
            {
                var page = await api.EditsAfter(chatId, seq, PageSize, ct).ConfigureAwait(false);
                if (!page.Ok || page.Value is null)
                {
                    return report with { Stopped = page.Error };
                }
                var edited = page.Value.Messages ?? [];
                if (edited.Length == 0)
                {
                    break;
                }
                chats.Apply(edited);
                report = report with { Edits = report.Edits + edited.Length };
                // A message in this feed carries the seq that put it here — an absent one is a
                // server breaking its own contract, and reads as 0, which cannot move the cursor
                // and so ends the loop rather than spinning it.
                var next = edited.Max(message => message.EditSeq ?? 0);
                if (next <= seq)
                {
                    break;
                }
                seq = next;
                chats.Advance(chatId, editSeq: seq);
                if (edited.Length < PageSize)
                {
                    break;
                }
            }
        }

        var pollCursor = chats.Chat(chatId)?.MaxPollSeq ?? 0;
        if ((row.MaxPollSeq ?? 0) > pollCursor)
        {
            var seq = pollCursor;
            while (true)
            {
                var page = await api.PollsAfter(chatId, seq, PageSize, ct).ConfigureAwait(false);
                if (!page.Ok || page.Value is null)
                {
                    return report with { Stopped = page.Error };
                }
                var states = page.Value.Polls ?? [];
                if (states.Length == 0)
                {
                    break;
                }
                foreach (var state in states)
                {
                    chats.ApplyPoll(chatId, state.MessageId, state.Poll, SeqRoute.CatchUpPage);
                }
                report = report with { Polls = report.Polls + states.Length };
                var next = states.Max(state => state.Poll.PollSeq);
                if (next <= seq)
                {
                    break;
                }
                seq = next;
                if (states.Length < PageSize)
                {
                    break;
                }
            }
        }
        return report;
    }

    /// <summary>
    /// The wall: the CHANGES feed from where this device left off, or the whole board when it has
    /// never read one — a full read REPLACES what is held, which is the only way a note somebody
    /// took down while this device was away actually leaves the wall.
    /// </summary>
    private async Task<Report> BoardAsync(long? mark, CancellationToken ct)
    {
        var report = new Report();
        // `max_board_seq` is ABSENT while the wall is empty and untouched, and that is the whole
        // point of the field: there is nothing to read, so nothing is asked for. It is checked
        // against what is HELD as well, because a wall this device holds notes for cannot be one
        // nobody has ever touched — and if the two ever disagree, reading wins.
        if (mark is null && board.Notes().Count == 0)
        {
            return report;
        }
        if (mark is not null && board.Cursor != 0 && mark <= board.Cursor)
        {
            // Level with the server: this device has applied every change there has been.
            return report;
        }
        if (board.Cursor == 0)
        {
            var whole = await api.Board(ct).ConfigureAwait(false);
            if (!whole.Ok || whole.Value is null)
            {
                return report with { Stopped = whole.Error };
            }
            var wall = whole.Value.Notes ?? [];
            board.Replace(wall, whole.Value.MaxBoardSeq);
            return report with { Notes = wall.Length };
        }
        while (true)
        {
            var page = await api.BoardChanges(board.Cursor, PageSize, ct).ConfigureAwait(false);
            if (!page.Ok || page.Value is null)
            {
                return report with { Stopped = page.Error };
            }
            var notes = page.Value.Notes ?? [];
            if (notes.Length == 0)
            {
                break;
            }
            // The board's own cursor is advanced by the store as it applies, because the feed
            // INCLUDES tombstones and a delete moves the seq like anything else.
            var was = board.Cursor;
            board.Apply(notes);
            report = report with { Notes = report.Notes + notes.Length };
            if (board.Cursor <= was || notes.Length < PageSize)
            {
                break;
            }
        }
        return report;
    }
}
