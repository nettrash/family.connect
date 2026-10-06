namespace FamilyConnect.Core.Store;

/// <summary>
/// Recordings THIS DEVICE has played, by attachment, in one table per kind — what the small accent dot beside an unplayed
/// one is drawn from, and nothing else (docs/audio-video-messages-2026-10-04.md, S5.2; the voice note's dot is the same
/// rule, drawn from the approved design of 2026-10-05).
/// </summary>
/// <remarks>
/// <para>
/// <b>THE DEVICE'S OWN KNOWLEDGE.</b> Never sent: whether somebody listened to or watched something is theirs, and the wire
/// has no field for it. Kept in the cache file, so it is this account's and goes with everything else at sign-out
/// (<see cref="Database.WipeAll"/>).
/// </para>
/// <para>
/// <b>THE NEWEST <see cref="Kept"/>, AND NO MORE.</b> Attachment ids only grow, so the highest ids are the newest;
/// marking one more past the limit lets the oldest go — which draws a dot again on something nobody will scroll back to.
/// </para>
/// </remarks>
public abstract class PlayedStore(Database database, string table)
{
    /// <summary>How many played recordings of one kind are remembered (S5.2).</summary>
    public const int Kept = 5000;

    /// <summary>Whether this device has played this recording.</summary>
    public bool Played(long attachmentId)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText = $"SELECT 1 FROM {table} WHERE attachment_id = $id";
        command.Parameters.AddWithValue("$id", attachmentId);
        return command.ExecuteScalar() is not null;
    }

    /// <summary>
    /// Remember that this recording was played here, and forget the oldest past <see cref="Kept"/>. Answers whether it was
    /// news — false when it had been played already, so a caller redraws only what changed.
    /// </summary>
    public bool MarkPlayed(long attachmentId)
    {
        using var serialised = database.Hold();
        using var transaction = database.Connection.BeginTransaction();
        using var insert = database.Connection.CreateCommand();
        insert.Transaction = transaction;
        insert.CommandText = $"INSERT OR IGNORE INTO {table} (attachment_id) VALUES ($id)";
        insert.Parameters.AddWithValue("$id", attachmentId);
        var added = insert.ExecuteNonQuery() > 0;
        if (added)
        {
            using var trim = database.Connection.CreateCommand();
            trim.Transaction = transaction;
            trim.CommandText =
                $"DELETE FROM {table} WHERE attachment_id NOT IN " +
                $"(SELECT attachment_id FROM {table} ORDER BY attachment_id DESC LIMIT $kept)";
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
            command.CommandText = $"SELECT COUNT(*) FROM {table}";
            return Convert.ToInt32(command.ExecuteScalar(), System.Globalization.CultureInfo.InvariantCulture);
        }
    }
}

/// <summary>The video messages this device has played: the dot beside an unplayed circle (S5.2).</summary>
public sealed class PlayedRoundStore(Database database) : PlayedStore(database, "played_rounds");

/// <summary>
/// The voice messages this device has played: the dot beside an unplayed voice bubble — never on the reader's own, which the
/// caller decides (<c>VoiceLook.ShowsDot</c>). A table of its own, so a busy chat of circles cannot push the voice notes out.
/// </summary>
public sealed class PlayedVoiceStore(Database database) : PlayedStore(database, "played_voice");
