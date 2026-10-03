namespace FamilyConnect.Core.Store;

/// <summary>One recording's text, as this device was given it.</summary>
/// <param name="AttachmentId">The recording: an attachment id never names other bytes, so this cannot go stale.</param>
/// <param name="Text">What was said. <c>""</c> is silence — an answer, drawn as "No speech".</param>
/// <param name="Language">The language the provider named, as it spelled it, or null when it named none.</param>
/// <param name="Supplied">Made from sound THIS DEVICE sent, which the server never kept: this row is the only copy.</param>
public sealed record KeptTranscript(long AttachmentId, string Text, string? Language = null, bool Supplied = false);

/// <summary>
/// The text of recordings this member asked for, kept on this device (docs/protocol.md, "Transcripts on request") — so
/// reopening a chat shows what was already given without asking the server again.
/// </summary>
/// <remarks>
/// <para>
/// <b>THIS DEVICE'S, AND NOBODY ELSE'S.</b> A transcript is the answer to one member's request: it is in no
/// <c>Attachment</c>, no history page and no frame, so nothing the wire delivers can fill or empty this table. It goes
/// with everything else at sign-out (<see cref="Database.WipeAll"/>), because the text of somebody's voice belongs to the
/// account that asked for it.
/// </para>
/// <para>
/// <b>AN ANSWER FROM THE SERVER'S STORED BYTES IS NEVER REPLACED BY ONE FROM SUPPLIED SOUND.</b> The stored one is the
/// answer every member gets; the server itself returns it instead of reading supplied sound once it exists.
/// </para>
/// </remarks>
public sealed class TranscriptStore(Database database)
{
    /// <summary>The text kept for this recording, or null when this device was never given any.</summary>
    public KeptTranscript? Find(long attachmentId)
    {
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "SELECT text, language, supplied FROM transcripts WHERE attachment_id = $id";
        command.Parameters.AddWithValue("$id", attachmentId);
        using var reader = command.ExecuteReader();
        if (!reader.Read())
        {
            return null;
        }
        return new KeptTranscript(
            attachmentId,
            reader.GetString(0),
            reader.IsDBNull(1) ? null : reader.GetString(1),
            reader.GetInt64(2) != 0);
    }

    /// <summary>Keep this answer. An answer from the stored bytes already held is not replaced by a supplied one.</summary>
    public void Keep(KeptTranscript transcript, DateTimeOffset at)
    {
        ArgumentNullException.ThrowIfNull(transcript);
        using var serialised = database.Hold();
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            "INSERT INTO transcripts (attachment_id, text, language, supplied, kept_at) " +
            "VALUES ($id, $text, $language, $supplied, $at) " +
            "ON CONFLICT(attachment_id) DO UPDATE SET text = excluded.text, language = excluded.language, " +
            "supplied = excluded.supplied, kept_at = excluded.kept_at " +
            "WHERE transcripts.supplied = 1 OR excluded.supplied = 0";
        command.Parameters.AddWithValue("$id", transcript.AttachmentId);
        command.Parameters.AddWithValue("$text", transcript.Text);
        command.Parameters.AddWithValue("$language", (object?)transcript.Language ?? DBNull.Value);
        command.Parameters.AddWithValue("$supplied", transcript.Supplied ? 1 : 0);
        command.Parameters.AddWithValue("$at", at.ToUnixTimeMilliseconds());
        command.ExecuteNonQuery();
    }
}
