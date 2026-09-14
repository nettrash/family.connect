using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What leaving means for the person leaving.</summary>
public enum LeaveKind
{
    /// <summary>A member: they lose the chats, and their history returns if they rejoin.</summary>
    Member,

    /// <summary>The owner, and somebody inherits.</summary>
    Successor,

    /// <summary>The owner and the last member: leaving DELETES the family.</summary>
    LastMember,
}

/// <summary>The leave dialog's question, built from a FRESH roster.</summary>
public sealed record LeaveContext(LeaveKind Kind, string? SuccessorName = null)
{
    /// <summary>
    /// The web client's <c>leave_context</c>. A successor the fresh roster does not name is NO
    /// answer (null): the dialog says leaving failed rather than guess — and a read that FAILED is
    /// not a family with nobody left in it, so "leaving deletes the family" is said only when the
    /// server said so.
    /// </summary>
    public static LeaveContext? For(bool owner, bool deletesFamily, MemberDto? successor) =>
        !owner ? new LeaveContext(LeaveKind.Member)
        : deletesFamily ? new LeaveContext(LeaveKind.LastMember)
        : successor is { } heir ? new LeaveContext(LeaveKind.Successor, heir.DisplayName)
        : null;
}

/// <summary>A birthday as the server takes it: a day and a month, no year.</summary>
public static class BirthdayRules
{
    /// <summary>February has 29: a birthday has no year for the 29th to fail to exist in.</summary>
    public static int DaysIn(int month) => month switch
    {
        2 => 29,
        4 or 6 or 9 or 11 => 30,
        _ => 31,
    };

    public static bool Ok(int month, int day) => month is >= 1 and <= 12 && day >= 1 && day <= DaysIn(month);

    /// <summary>Where the picker opens: unset on the 1st of January, a held one on itself, held inside the calendar.</summary>
    public static (int Month, int Day) Start(BirthdayDto? held)
    {
        if (held is null)
        {
            return (1, 1);
        }
        var month = Math.Clamp(held.Month, 1, 12);
        return (month, Math.Clamp(held.Day, 1, DaysIn(month)));
    }

    /// <summary>"March 12", "12 марта" — the reader's own way of writing a day of a month.</summary>
    public static string Text(BirthdayDto? birthday, IStringCatalog say, CultureInfo culture) =>
        birthday is { } held && Ok(held.Month, held.Day)
            ? new DateTime(2024, held.Month, held.Day).ToString("M", culture)
            : say.Get("Not set");

    /// <summary>A month standing alone, as a picker lists it: "January", "январь".</summary>
    public static string MonthName(int month, CultureInfo culture) =>
        culture.DateTimeFormat.GetMonthName(month);
}

/// <summary>
/// The words the settings screen says — each the web client's (<c>web/src/views/settings.rs</c>,
/// <c>password.rs</c>, <c>birthday.rs</c>), which are the Mac's.
/// </summary>
public static class SettingsText
{
    /// <summary>The fewest characters a password may have, counted in SCALARS as the server counts.</summary>
    public const int MinPasswordChars = 8;

    public static string LeaveMessage(LeaveContext context, IStringCatalog say) => context.Kind switch
    {
        LeaveKind.Successor => say.Format(
            "%@ becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin.",
            context.SuccessorName ?? string.Empty),
        LeaveKind.LastMember => say.Get("You're the only member left. Leaving deletes the family and everything in it."),
        _ => say.Get("You'll lose access to the family chat and your direct chats. Your history returns if you rejoin."),
    };

    /// <summary>Leaving as the last member DELETES the family: a different question, and a different button.</summary>
    public static string LeaveTitle(LeaveContext context, IStringCatalog say) =>
        context.Kind == LeaveKind.LastMember ? say.Get("Delete the family?") : say.Get("Leave the family?");

    public static string LeaveButton(LeaveContext context, IStringCatalog say) =>
        context.Kind == LeaveKind.LastMember ? say.Get("Leave and Delete") : say.Get("Leave Family");

    public static string LeaveFailed(IStringCatalog say) => say.Get("Couldn't leave right now. Try again.");

    /// <summary>Why a picture did not go up, or come down (ios AvatarFailure).</summary>
    public static string PictureFailure(ApiError error, bool uploading, IStringCatalog say) => error switch
    {
        { Code: ErrorCodes.AvatarTooLarge } or { Status: 413 } => say.Get("That photo is too large for this server."),
        { Code: ErrorCodes.InvalidImage } => say.Get("That file isn't a photo we can use."),
        { Code: ErrorCodes.Transport } => say.Get("Can't reach the server. Check your connection."),
        { Code: ErrorCodes.TooManyRequests } or { Status: 429 } => say.Get("The server is busy. Try again in a moment."),
        _ => uploading ? say.Get("Couldn't upload the photo.") : say.Get("Couldn't remove the photo."),
    };

    public static string DeleteFailure(ApiError error, IStringCatalog say) => error.Code switch
    {
        ErrorCodes.InvalidCredentials => say.Get("That password is not right."),
        ErrorCodes.Validation => say.Get("Type your password to confirm."),
        _ => say.Get("Couldn't delete your account. Try again."),
    };

    /// <summary>What is wrong with a new password and its confirmation, or null.</summary>
    public static string? PasswordProblem(string newPassword, string confirmation, IStringCatalog say)
    {
        if (newPassword.EnumerateRunes().Count() < MinPasswordChars)
        {
            return say.Format("Use at least %lld characters.", MinPasswordChars);
        }
        return string.Equals(newPassword, confirmation, StringComparison.Ordinal)
            ? null
            : say.Get("Those two do not match.");
    }

    public static string PasswordChangeFailure(ApiError error, IStringCatalog say) =>
        error.Code == ErrorCodes.InvalidCredentials
            ? say.Get("That current password is not right.")
            : say.Get("Couldn't change your password. Try again.");

    public static string BirthdayFailure(ApiError error, IStringCatalog say) => error.Code switch
    {
        ErrorCodes.Validation => say.Get("That date doesn't exist."),
        ErrorCodes.NotFamilyOwner => say.Get("Only the family owner can do that."),
        _ => say.Get("Couldn't save that birthday. Try again."),
    };

    /// <summary>What one member sends, besides words (ios StatisticsView).</summary>
    public static string MemberLine(StatsMemberDto member, IStringCatalog say, CultureInfo? culture = null)
    {
        var parts = new List<string>();
        if (member.Attachments is { Count: > 0 } files)
        {
            // The count chooses the form; the arguments are the key's own, and a translation may
            // say the size first.
            parts.Add(say.Plural(
                "%lld attachments, %@", files.Count, files.Count,
                MediaText.DisplaySize(Math.Max(0, files.Bytes), say, culture)));
        }
        if (member.Ai is { Questions: > 0 } questions)
        {
            parts.Add(say.Plural("%lld questions to the assistant", questions.Questions, questions.Questions));
        }
        if (member.Ai is { Images: > 0 } images)
        {
            parts.Add(say.Plural("%lld pictures from the assistant", images.Images, images.Images));
        }
        return parts.Count == 0 ? say.Get("Words only") : string.Join(" · ", parts);
    }

    /// <summary>
    /// What storing one copy of identical files saved, or null when it saved nothing — the family's
    /// own totals, never the rows added up: the gap between those is the reader's block list.
    /// </summary>
    public static long? Saved(StatsMediaDto? files)
    {
        if (files?.StoredBytes is not { } stored)
        {
            return null;
        }
        var saved = Math.Max(0, files.Bytes) - Math.Max(0, stored);
        return saved > 0 ? saved : null;
    }
}
