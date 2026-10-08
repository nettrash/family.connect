using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What a message IS when it is a recording and nothing else: a voice message, or a video message.</summary>
public enum RecordingKind
{
    Voice,
    Round,
}

/// <summary>One item of a message's context menu, before the Safety group (Report…, Block), which follows them all.</summary>
public enum MessageAction
{
    Reply,
    React,
    ShowText,
    PlaybackSpeed,
    Save,
    OpenFullScreen,
    Copy,
    ViewThread,
    Edit,
}

/// <summary>What the menu's text item is for a recording's text as it stands (<see cref="MessageMenu.TextItemFor"/>).</summary>
public enum TextItem
{
    /// <summary>No item: the text is being asked for, or could not be had and asking again would not help.</summary>
    None,

    /// <summary>"Show text": asks for it — as the words under the recording do.</summary>
    Show,

    /// <summary>"Hide text": folds it away.</summary>
    Hide,
}

/// <summary>
/// A message's context menu, item by item (the approved design of 2026-10-05: "Long-press menu on a voice or video
/// message"). A RECORDING has nothing to copy, edit or select — its menu is the reactions first, then Reply, the thread,
/// Show text, Playback speed for a voice message, Save… and Open Full Screen for a video message (the order every client
/// draws) — and every other message's menu is exactly what it always was.
/// </summary>
public static class MessageMenu
{
    /// <summary>
    /// Whether this message is a recording and nothing else: a video message (<see cref="MessageDto.RoundVideo"/>, the shared
    /// test), or exactly one sound with NO BODY. A recording sent with words is a message with words — Copy and Edit stay.
    /// </summary>
    public static RecordingKind? Recording(MessageDto message) =>
        message.RoundVideo is not null ? RecordingKind.Round
        : string.IsNullOrEmpty(message.Body) && message.Poll is null && message.Call is null
            && message.Media is [{ Kind: "audio" }]
            ? RecordingKind.Voice
            : null;

    /// <summary>
    /// The text item for a recording whose text stands at <paramref name="look"/> — offered EXACTLY where the words under the
    /// recording offer the same thing: "Hide text" while it is open, "Show text" while it is folded away or after a failure
    /// that trying again could help (the row's "Try Again"), and nothing while it is being asked for ("Getting the text…") or
    /// after a failure the row deliberately offers no way past.
    /// </summary>
    public static TextItem TextItemFor(TranscriptLook look)
    {
        ArgumentNullException.ThrowIfNull(look);
        return look.Phase switch
        {
            TranscriptPhase.Open => TextItem.Hide,
            TranscriptPhase.Asking => TextItem.None,
            TranscriptPhase.Failed => TranscriptRules.MayRetry(look.Error ?? ApiError.Transport("no answer"))
                ? TextItem.Show
                : TextItem.None,
            _ => TextItem.Show,
        };
    }

    /// <summary>The items, in order.</summary>
    /// <param name="message">The message the menu is for.</param>
    /// <param name="mayEdit">Whether the reader may edit it (<see cref="ConversationModel.MayEdit"/>).</param>
    /// <param name="inThread">The menu is the thread panel's, where Edit and View thread are not offered.</param>
    /// <param name="offersText">"Show text" can be asked for here — the recording's place for its text is drawn.</param>
    public static IReadOnlyList<MessageAction> Actions(MessageDto message, bool mayEdit, bool inThread, bool offersText)
    {
        var chained = !inThread && (message.ThreadRootId is not null || message.ReplyCount is not null);
        if (Recording(message) is { } kind)
        {
            var items = new List<MessageAction> { MessageAction.React, MessageAction.Reply };
            // The thread straight after Reply, where iOS, the Mac, Android and the web put it on a recording's menu.
            if (chained)
            {
                items.Add(MessageAction.ViewThread);
            }
            if (offersText)
            {
                items.Add(MessageAction.ShowText);
            }
            if (kind == RecordingKind.Voice)
            {
                items.Add(MessageAction.PlaybackSpeed);
            }
            items.Add(MessageAction.Save);
            if (kind == RecordingKind.Round)
            {
                items.Add(MessageAction.OpenFullScreen);
            }
            return items;
        }
        var ordinary = new List<MessageAction> { MessageAction.Reply, MessageAction.React };
        if (message.Body.Length > 0 && message.Call is null)
        {
            ordinary.Add(MessageAction.Copy);
        }
        if (chained)
        {
            ordinary.Add(MessageAction.ViewThread);
        }
        if (!inThread && mayEdit)
        {
            ordinary.Add(MessageAction.Edit);
        }
        return ordinary;
    }
}
