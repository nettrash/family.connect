namespace FamilyConnect.Core.Store;

/// <summary>
/// The video messages THIS DEVICE has played (docs/audio-video-messages-2026-10-04.md, S5.2) — what the small accent dot
/// beside an unplayed circle is drawn from, and nothing else.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE DEVICE'S OWN KNOWLEDGE.</b> Never sent: whether somebody watched a video is theirs, and the wire has no field for
/// it. Kept in the cache file, so it is this account's and goes with everything else at sign-out
/// (<see cref="Database.WipeAll"/>).
/// </para>
/// <para>
/// <b>THE NEWEST <see cref="Kept"/>, AND NO MORE.</b> Attachment ids only grow, so the highest ids are the newest videos;
/// marking one more past the limit lets the oldest go — which draws a dot again on a circle nobody will scroll back to.
/// </para>
/// </remarks>
public sealed class PlayedRoundStore(Database database)
{
    /// <summary>How many played circles are remembered (S5.2).</summary>
    public const int Kept = 5000;

    /// <summary>Whether this device has played this video message.</summary>
    public bool Played(long attachmentId)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText = "SELECT 1 FROM played_rounds WHERE attachment_id = $id";
        command.Parameters.AddWithValue("$id", attachmentId);
        return command.ExecuteScalar() is not null;
    }

    /// <summary>
    /// Remember that this video message was played here, and forget the oldest past <see cref="Kept"/>. Answers whether
    /// it was news — false when it had been played already, so a caller redraws only what changed.
    /// </summary>
    public bool MarkPlayed(long attachmentId)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        using var insert = database.Connection.CreateCommand();
        insert.Transaction = transaction;
        insert.CommandText = "INSERT OR IGNORE INTO played_rounds (attachment_id) VALUES ($id)";
        insert.Parameters.AddWithValue("$id", attachmentId);
        var added = insert.ExecuteNonQuery() > 0;
        if (added)
        {
            using var trim = database.Connection.CreateCommand();
            trim.Transaction = transaction;
            trim.CommandText =
                "DELETE FROM played_rounds WHERE attachment_id NOT IN " +
                "(SELECT attachment_id FROM played_rounds ORDER BY attachment_id DESC LIMIT $kept)";
            trim.Parameters.AddWithValue("$kept", Kept);
            trim.ExecuteNonQuery();
        }
        transaction.Commit();
        return added;
    }

    /// <summary>How many are remembered — never more than <see cref="Kept"/>.</summary>
    public int Count
    {
        get
        {
            using var serialised = database.Hold();
            using var command = database.Connection.CreateCommand();
            command.CommandText = "SELECT COUNT(*) FROM played_rounds";
            return Convert.ToInt32(command.ExecuteScalar(), System.Globalization.CultureInfo.InvariantCulture);
        }
    }
}
