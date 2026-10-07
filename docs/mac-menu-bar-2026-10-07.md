# The Mac in the menu bar (issue #80)

**Written 2026-10-07** against `v1.2`, in answer to issue #80: *"macOS client should run on watch tray
as we use for windows — the main idea is to have only tray icon with notifications, and open window by
clicking notifications or by clicking tray icon or by shortcut key."*

## What Windows does, and why it matters more than it looks

Windows closes to the notification area (`win/README.md`, "SO CLOSING THE WINDOW DOES NOT QUIT"): with
no push to wake it, a client that stops listening when its window closes misses the very call it exists
to ring for. The icon opens the window (a click) or quits it (its menu); "Keep running when the window is
closed" is on by default; "Start when I sign in" starts it with no window at all.

The Mac had the same hole, hidden. A Mac app survives its last window, but `RootView` — the view that
opens the call window and drives the scene phase — lives INSIDE that window, and a window that closes,
hides or minimises takes the scene to `.background`, where `ChatSyncCoordinator.enterBackground`
suspends the socket (`SocketHold`). APNs still brings the Mac a message banner, but protocol.md
("Incoming calls") wakes no Mac for a call: **a Mac with its window closed or minimised could not ring.**

## The design

1. **A menu bar icon** (`MacMenuBar`, an `NSStatusItem` — not a `MenuBarExtra`, which can only open a menu
   or a panel on a click). A click opens the window; a right-click or Control-click opens a menu: *Open
   Family Connect*, *Settings…*, *Quit Family Connect*. The unread count stands beside the icon (the Dock
   badge goes with the Dock icon), and its tooltip is the window title's "(3) Family Connect".
2. **Closing HIDES the main window** — its close button and ⌘W — so `RootView` stays alive, the call window
   still opens on a ring, a notification's route is still applied, and the socket stays up. Only the main
   window, and only the last one (a second File ▸ New Window closes, so the menu bar never holds two
   hidden copies); a conversation, the board, an attachment, Settings close as before. A hidden window is a window
   that went away for recording purposes (`VoiceRecordingArbiter.windowWentAway`, S8.3 of #79), exactly as
   Windows' `RecordingEnd.WindowHidden`.
3. **The Dock icon only while a window is open.** With no window on the screen the app is an *accessory*
   (`NSApp.setActivationPolicy(.accessory)`): no Dock icon, no menu bar of its own, not in ⌘-Tab — the
   menu bar icon is the app. Any window coming up — the main one, Settings, the call — makes it a regular
   app again, so ⌘-Tab, ⌘Q and every menu command work whenever there is something to use them on. A
   minimised window counts as open (the Dock is how you get it back), and so does everything while the app
   is hidden with ⌘H.
4. **The socket stays up in the background** while the app keeps running in the menu bar
   (`SocketHold.decide(..., listensInBackground:)`), as Windows' does. The server then pushes nothing to
   this Mac (it has a live socket) and `ChatNotifier` raises the banners itself, as it already does for a
   window that is open but not in front. protocol.md is amended to say a Mac kept in the menu bar rings.
5. **A global shortcut, ⌃⌥⌘F** (F for Family), registered with Carbon's `RegisterEventHotKey` — the one
   system-wide shortcut API that needs neither Accessibility permission nor a hole in the sandbox. It
   brings the window forward from any app; pressed while the window is in front, it puts it back in the
   menu bar. On by default, switchable; if another app already holds the combination, Settings says so.
   A fixed combination rather than a recorder: one well-chosen chord is the whole feature, and a shortcut
   recorder is a control of its own (a follow-up if anybody asks for another key).
6. **Open at login** (`SMAppService.mainApp`), off until asked for. A launch macOS made at login (the open
   event's `keyAELaunchedAsLogInItem`) starts in the menu bar with its window hidden — only when the menu
   bar icon is on, as Windows' `StartsHidden` refuses to hide a window there is no icon to come back from.
   `.requiresApproval` (switched off in System Settings) is shown as such, with a button to Login Items —
   Windows' `DisabledByUser`.
7. **Clicking a notification** brings the main window up first (creating one if it was really closed), then
   parks the route as before; *Answer* on a call banner brings it up too, so an answered call always has
   its window.

### Settings ▸ Menu Bar (Mac only)

- **Keep Running in the Menu Bar** — on. *"When you close the window, Family Connect stays in the menu
  bar, so messages and calls still reach you. Quit it from its icon there."* Off is the Mac app as it
  was: a Dock app whose closed window is closed.
- **Open at Login** — off. *"Family Connect opens in the menu bar when you log in to this Mac."*
- **Open with ⌃⌥⌘F** — on. *"Brings Family Connect forward from any app. Press it again to put it back in
  the menu bar."*

All three are this Mac's, not the account's: they are not wiped at sign-out (like link and map previews).

### What is pure, and tested

`MenuBarRules`: the activation policy for a set of windows, whether a close hides, whether a launch starts
hidden, what the hot key does, the icon's count text, and what a login item's status means for its
switch. `SocketHold.decide` gains its third input. Everything that touches AppKit — the status item, the
policy switch, the close interception, the hot key, `SMAppService` — is thin wiring around them.

## Windows gets the shortcut too

Windows had the icon but no shortcut. It now has the same chord in its own spelling — **Ctrl+Alt+Shift+F**
(`GlobalHotKey`, `RegisterHotKey`; `GlobalHotKeyRules` in App.Logic, tested), with the same behaviour: the
window from anywhere, and back into the notification area when pressed while it is in front (only where a
close would keep the app running there). Three modifiers, as on the Mac, and never Ctrl+Alt alone — that
is AltGr, and AltGr+F TYPES "[" on Hungarian and Czech keyboards. "Open with Ctrl+Alt+Shift+F" in
Settings, on by default; "Another app is using Ctrl+Alt+Shift+F." when it is taken.

## The Mac's own notifications, as Windows' (issue #84)

A Mac with its window open raises its own banners from the live socket (the server pushes nothing to a
device whose socket is live), and #84 found two Macs it never reached: one macOS had never been allowed
to show the app's notifications, or showed them as None — every banner raised into nothing, with no word
anywhere about why. The Mac now follows the Windows client's `NotificationRules`
(`DesktopNotificationRules`, tested on both runs): the banner says who wrote — "<Family> — <Sender>",
or "… mentioned you" — over "New message", never the words; a new board note is announced too ("New
note", only when the board badge calls it new, and not while the board window is in front); nothing
from a blocked member or the assistant's answer to one; and Settings ▸ Notifications has "Tell me when a
message arrives" (on by default, this Mac's), with a line under it when macOS has them off or set to
None and a button to System Settings ▸ Notifications. Every decision is logged by id with its reason
(`log stream --predicate 'subsystem == "me.nettrash.FamilyConnect" AND category == "push"'`), so a
silent Mac can say why. The mention title was also English in every language; it is translated now.

Two holes the PR review (#86) found, both about the pushes a QUIT Mac gets from the server, now that its push
entitlement survives signing: the switch only silenced the app's own banners, and "never what they wrote" was
the server's `include_message_body` to keep. So switching notifications off now WITHDRAWS this Mac's device
from the server (`PushRegistrar.withdraw`, forgotten here only once the server has, retried at the next
launch if it could not be reached) and switching them on registers it again; and the server pushes a `macos`
device the body `include_message_body = false` would send, whatever the setting (protocol.md, "A Mac is
pushed who, never what"). A Mac now says the same thing whether its app was running or quit.

## What must be checked on a real Mac (none of it can run unsigned)

- Closing the window leaves the menu bar icon and no Dock icon; a click brings the window back where it was,
  with the conversation it had open; ⌘Q from the icon's menu quits.
- A call from a phone while the window is closed rings: banner, then the call window; Answer on the
  banner opens the call window.
- A message while the window is closed shows a banner; clicking it opens the window on that chat.
- ⌃⌥⌘F from another app opens the window; again, hides it.
- Open at Login: log out and in — the icon is there, no window, and a message still arrives.
- The unread count beside the icon matches the chat list.
