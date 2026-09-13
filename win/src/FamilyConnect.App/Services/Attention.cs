using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Services;

/// <summary>
/// What asks for the reader's attention while the app runs: a notification for a live message or a
/// note that says something new, and the unread count in the window's title and on its taskbar icon.
/// </summary>
/// <remarks>
/// Only LIVE frames raise anything — the router raises <c>Arrived</c> and <c>BoardChanged</c> for
/// those alone — so a resync that brings in a hundred messages after a night asleep is a badge, not
/// a hundred notifications.
/// </remarks>
internal sealed class Attention : IDisposable
{
    private readonly AppServices services;
    private readonly Connection connection;
    private readonly NotificationRules rules;
    private readonly Action changed;

    /// <param name="changed">The count may have moved. Raised on any thread; the caller marshals.</param>
    public Attention(AppServices services, Connection connection, Action changed)
    {
        this.services = services;
        this.connection = connection;
        this.changed = changed;
        rules = new NotificationRules(connection.Chats, services.Say);
        connection.Router.Arrived += OnArrived;
        connection.Router.BoardChanged += OnNote;
        connection.Router.ChatChanged += OnChat;
        connection.Live.Resynced += OnResync;
    }

    /// <summary>The window's title: the count, then the name.</summary>
    public string Title => rules.WindowTitle("Family Connect");

    public int Unread => connection.Chats.Unread();

    private void Prime()
    {
        var state = connection.Session.State;
        rules.FamilyName = state.Family?.Name;
        rules.AssistantUserId = state.Assistant?.UserId;
        rules.InFront = services.Foreground;
        rules.Wanted = Toasts.Available;
    }

    private void OnArrived(MessageDto message)
    {
        Prime();
        if (rules.ForMessage(message) is { } toast)
        {
            Toasts.Show(toast);
        }
        changed();
    }

    private void OnNote(NoteDto note)
    {
        Prime();
        if (rules.ForNote(note, ToastArguments.IsNews(note, connection.Board.Marks)) is { } toast)
        {
            Toasts.Show(toast);
        }
    }

    private void OnChat(long chatId) => changed();

    private void OnResync(Resync.Report report) => changed();

    public void Dispose()
    {
        connection.Router.Arrived -= OnArrived;
        connection.Router.BoardChanged -= OnNote;
        connection.Router.ChatChanged -= OnChat;
        connection.Live.Resynced -= OnResync;
    }
}
