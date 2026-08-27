/*
 * ChatScreen.kt
 * Family Connect (Android)
 *
 * The conversation. reverseLayout LazyColumn (index 0 = newest = bottom)
 * keyed by clientMsgId — the key survives the ack because ack UPDATEs
 * the row instead of replacing it. Bubbles: one Surface for both sides
 * — mine end-aligned on primaryContainer, theirs start-aligned on
 * surfaceContainerHigh — with 18dp corners tightened to 4dp against
 * same-sender run neighbors, capped at 80% of the row width.
 * Emoji-only messages render bare: transparent balloon, glyphs on the
 * EmojiOnly size ladder (identical on iOS). Text bodies go through
 * MessageLinks: URLs, emails and phone numbers render as tappable
 * links (browser / mail / dialer).
 * Status glyphs on my bubbles: clock (sending), ✓ (sent), ✓✓ (read —
 * direct chats only, serverId ≤ peerLastReadId), red error → retry/
 * delete dialog. Date pills between days; typing shows on a permanently
 * reserved app-bar subtitle line so the bar never changes height.
 * Scrolling within 10 items of the old end triggers loadOlder (spinner
 * in the list's oldest-end item); a small FAB overlaid above the input
 * bar jumps back to the newest message.
 * OPENING: a chat with anything unread opens ANCHORED at the oldest
 * unread message, under an "N new messages" divider, instead of at the
 * bottom. The decision is the ViewModel's (OpenAnchor.kt) and is taken
 * once; this file only performs it, through the same bounded
 * page-and-scroll pipeline a quote tap uses — pendingJump searches and
 * pages older, scrollToJump moves. An anchored open reads NOTHING:
 * the bottom is off screen, so the read collector's at-newest gate is
 * false, and until the move finishes `settled` holds that gate shut
 * whatever the empty list's index 0 claims. Both opening branches end
 * in viewModel.setSettled(), and everything that could yank the reader
 * back to the bottom (the arrival follow rule, the near-old-end
 * pagination trigger) waits for it.
 * Reactions: chips under the bubble (tap toggles, long-press shows who
 * reacted); long-press on an acked bubble opens a floating capsule
 * anchored above it (below near the top) with the quick set + my
 * off-list reaction + a "+" into the full categorized emoji picker
 * sheet (EMOJI_CATALOG); double-tap on an acked bubble toggles the
 * quick heart (DOUBLE_TAP_REACTION).
 *
 * iOS counterpart: ios/FamilyConnect/Views/ConversationView.swift
 */

package me.nettrash.familyconnect.ui.chat

import android.content.ClipData
import android.content.Intent
import android.os.Build
import android.widget.Toast
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.TransferableContent
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.Image
import android.content.Context
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallMade
import androidx.compose.material.icons.filled.CallMissed
import androidx.compose.material.icons.filled.CallReceived
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.InsertDriveFile
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Poll
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.HowToVote
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.automirrored.outlined.Reply
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Forum
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.toClipEntry
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.LinkPreviewState
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.GallerySaver
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import android.net.Uri
import android.Manifest
import android.content.pm.PackageManager
import androidx.core.content.ContextCompat
import androidx.compose.material.icons.filled.Place
import me.nettrash.familyconnect.data.repo.LocationProvider
import androidx.compose.material.icons.filled.Mic
import android.graphics.BitmapFactory
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.ui.graphics.asImageBitmap
import me.nettrash.familyconnect.data.repo.MediaPrep
import androidx.core.content.FileProvider
import java.io.File
import me.nettrash.familyconnect.ui.components.AttachmentGroup
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
import me.nettrash.familyconnect.ui.components.EmptyState
import me.nettrash.familyconnect.ui.components.OfflineBanner
import me.nettrash.familyconnect.util.TimeFormat
import kotlin.math.roundToInt

/**
 * The long-pressed message plus its bubble's window bounds — the
 * floating reaction capsule anchors to those bounds. Not a one-shot
 * snapshot: the pressed bubble stays composed under the popup, and its
 * onPositioned keeps anchorBounds current while the capsule is open
 * (the focusable popup closes the keyboard, shifting the list).
 */
private data class ReactionPickerTarget(
    val item: ChatListItem.MessageItem,
    val anchorBounds: Rect,
)

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ChatScreen(
    onBack: () -> Unit,
    viewModel: ChatViewModel = hiltViewModel(),
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val typingUser by viewModel.typingUser.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    val memberAvatars by viewModel.memberAvatars.collectAsStateWithLifecycle()
    val callsEnabled by viewModel.callsEnabled.collectAsStateWithLifecycle()
    // Which rows the assistant is still writing into. In-memory only, so a
    // row that was mid-stream when the app was killed is not stuck looking
    // live after a relaunch.
    val streamingIds by viewModel.streamingMessageIds.collectAsStateWithLifecycle()
    // Null when the server has no assistant configured, which is what
    // decides whether the composer offers `@ai` at all.
    val assistantUserId by viewModel.assistantUserId.collectAsStateWithLifecycle()
    val isOnline by viewModel.isOnline.collectAsStateWithLifecycle()
    val socketState by viewModel.socketState.collectAsStateWithLifecycle()
    val loadingOlder by viewModel.loadingOlder.collectAsStateWithLifecycle()
    val initialLoadSettled by viewModel.initialLoadSettled.collectAsStateWithLifecycle()
    // Where this chat opens, and whether the opening is over. Null anchor
    // = still deciding; acting on it then would settle the screen before
    // there is anything to settle on.
    val openAnchor by viewModel.openAnchor.collectAsStateWithLifecycle()
    val settled by viewModel.settled.collectAsStateWithLifecycle()
    val linkPreviews by viewModel.linkPreviews.collectAsStateWithLifecycle()
    val linkPreviewsEnabled by viewModel.linkPreviewsEnabled.collectAsStateWithLifecycle()
    val mapPreviewsEnabled by viewModel.mapPreviewsEnabled.collectAsStateWithLifecycle()

    val mediaState by viewModel.mediaState.collectAsStateWithLifecycle()
    val staged by viewModel.staged.collectAsStateWithLifecycle()
    val recordingMs by viewModel.recordingMs.collectAsStateWithLifecycle()

    var failedActionTarget by remember { mutableStateOf<String?>(null) }

    // Leave when the chat itself goes.
    //
    // A direct chat can vanish under the reader: the peer deletes their
    // account, `member_deleted` lands, and ChatRepository drops the chat
    // and every message in it (docs/protocol.md, "Deleting an account" —
    // the frame reaches the peer precisely "because their chat is about to
    // vanish and nothing else would ever say why"). Nothing here reacted:
    // the title went blank, the thread emptied, and the composer stayed
    // live, so a message typed into it was stored against a chat that no
    // longer existed and came back FAILED with `chat_not_found` and no
    // explanation. Both Apple clients already step out of a chat that
    // disappears (ChatListView pops the pushed thread, MacChatView moves
    // the selection).
    //
    // Non-null → null, never plain null: `chat` is a StateFlow started
    // Eagerly with an initial null, so a first-frame `chat == null` is
    // Room not having answered yet and popping on it would close the
    // screen every time it opened.
    val currentOnBack by rememberUpdatedState(onBack)
    var chatWasSeen by remember { mutableStateOf(false) }
    LaunchedEffect(chat != null) {
        if (chat != null) chatWasSeen = true else if (chatWasSeen) currentOnBack()
    }

    // The attachment open full screen, and the picker that starts a send.
    var viewingAttachment by remember { mutableStateOf<AttachmentDto?>(null) }

    // The message the floating capsule is open for, and the one the "+"
    // full-picker sheet is open for. Both are transient snapshots.
    var pickerTarget by remember { mutableStateOf<ReactionPickerTarget?>(null) }
    var fullPickerTarget by remember { mutableStateOf<ChatListItem.MessageItem?>(null) }
    val listState = rememberLazyListState()
    // Set by tapping a quote, or by the opening anchor; cleared once the
    // target is found (or given up on). Held here rather than in the
    // ViewModel because it is purely a scroll intent — nothing about it
    // survives leaving the screen. The DECISION to anchor is the
    // ViewModel's and does survive, which is the difference.
    var pendingJump by remember { mutableStateOf<JumpRequest?>(null) }
    var highlightedMessageId by remember { mutableStateOf<Long?>(null) }
    var jumpPagesTried by remember { mutableIntStateOf(0) }
    var scrollToJump by remember { mutableStateOf<JumpRequest?>(null) }
    // Half the viewport, so animateScrollToItem's top-edge alignment lands
    // the target in the middle instead.
    val centreOffsetPx = with(LocalDensity.current) {
        (LocalConfiguration.current.screenHeightDp.dp / 2).roundToPx()
    }
    // How far below the top edge an anchored open lands the divider. The
    // margin is not decoration: land the target flush against the top and
    // the near-old-end band fires load-older, whose own window growth
    // fights the scroll that just happened. It also leaves room for the
    // divider itself on the pass where the row has not been built yet.
    val anchorTopMarginPx = with(LocalDensity.current) { ANCHOR_TOP_MARGIN.roundToPx() }
    val focusRequester = remember { FocusRequester() }
    val replyDraft by viewModel.replyDraft.collectAsStateWithLifecycle()
    val editTarget by viewModel.editTarget.collectAsStateWithLifecycle()
    // The poll being written, and whether this chat may hold one at all
    // (the family chat only — anywhere else the server says invalid_poll).
    val pollDraft by viewModel.pollDraft.collectAsStateWithLifecycle()
    val canCreatePoll by viewModel.canCreatePoll.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    // Copy and share from the message context menu.
    val clipboard = LocalClipboard.current
    // Hoisted: a coroutine is not a composable context.
    val clipLabel = stringResource(R.string.s_message)
    // Resolved out here: toasts and the attachment-busy notice fire from
    // click handlers and coroutines, which are not composable contexts.
    val copiedLabel = stringResource(R.string.s_copied)
    val savedToGalleryLabel = stringResource(R.string.s_saved_to_gallery)
    val preparingLabel = stringResource(R.string.s_preparing)
    val context = LocalContext.current

    // The system photo picker: no permission, no gallery access — it
    // hands back the picked Uris and nothing else, which is the whole
    // reason this app never asks for READ_MEDIA_IMAGES. Multiple since a
    // message learned to carry up to ten attachments; the picker itself
    // enforces the cap.
    val pickMedia = rememberLauncherForActivityResult(
        ActivityResultContracts.PickMultipleVisualMedia(AttachmentDto.MAX_PER_MESSAGE),
    ) { uris ->
        if (uris.isNotEmpty()) {
            val items = uris.map { uri ->
                // Guarded: `getType` reaches into the picker's provider and
                // throws for a Uri it does not recognise — on the main
                // thread, where that is a crash rather than a failed pick.
                val type = runCatching { context.contentResolver.getType(uri) }
                    .getOrNull()
                    .orEmpty()
                uri to type.startsWith("video/")
            }
            viewModel.stageMedia(items)
        }
    }
    // OpenMultipleDocuments, not GetContent: it returns Uris whose read
    // permission this process actually holds, which is what makes the copy
    // in MediaPrep.prepareFile work. "*/*" because the family is never told
    // what they may send (protocol.md, "Files"). The staging cap trims a
    // selection past ten with a notice.
    val pickFile = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments(),
    ) { uris -> if (uris.isNotEmpty()) viewModel.stageFiles(uris) }

    // Pasting. Two doors into the same staging: the attach menu's Paste,
    // which reads the clipboard itself and works with the field unfocused,
    // and the field's own paste gesture (below, via contentReceiver) —
    // which is also where Ctrl+V from a hardware keyboard, a keyboard that
    // inserts pictures, and a drop onto the composer arrive.
    //
    // Neither door decides anything: both hand the whole clipboard to the
    // ViewModel, which asks PastedMedia.decide what it is. What differs is
    // only what they do with WORDS — see pasteIntoField.
    //
    // Reading the clipboard is a suspend call — deliberately, since it can
    // reach across to another app's provider — hence the scope.
    val pasteFromClipboard: () -> Unit = {
        scope.launch { viewModel.pasteFromClipboard(clipboard.getClipEntry()?.clipData) }
    }
    val pasteContent: (TransferableContent) -> ChatViewModel.PasteResult = { transferable ->
        // The transferable itself is the keep-alive: content committed by a
        // keyboard carries a read grant that dies when its InputContentInfo
        // is collected, and the copy runs on another thread.
        viewModel.pasteIntoField(transferable.clipEntry.clipData, keepAlive = transferable)
    }

    // The camera. MediaStore's capture intents hand the picture back into a
    // Uri WE provide, via the FileProvider the app already declares for
    // opening downloaded files.
    //
    // Note what is deliberately absent: android.permission.CAMERA. The
    // intents are documented to throw SecurityException when an app DECLARES
    // that permission without holding it — so declaring it "to be safe" is
    // what breaks this, and not declaring it keeps the app's posture (no
    // camera access of its own, only what the user hands over).
    var pendingCapture by remember { mutableStateOf<Uri?>(null) }
    var pendingCaptureIsVideo by remember { mutableStateOf(false) }
    val takePicture = rememberLauncherForActivityResult(
        ActivityResultContracts.TakePicture(),
    ) { ok ->
        val uri = pendingCapture
        pendingCapture = null
        if (ok && uri != null) viewModel.stageMedia(uri, isVideo = false)
    }
    val captureVideo = rememberLauncherForActivityResult(
        ActivityResultContracts.CaptureVideo(),
    ) { ok ->
        val uri = pendingCapture
        pendingCapture = null
        if (ok && uri != null) viewModel.stageMedia(uri, isVideo = true)
    }
    val startCapture: (Boolean) -> Unit = { isVideo ->
        val uri = viewModel.newCaptureUri(context, isVideo)
        if (uri != null) {
            pendingCapture = uri
            pendingCaptureIsVideo = isVideo
            if (isVideo) captureVideo.launch(uri) else takePicture.launch(uri)
        }
    }

    // Recording a voice note. RECORD_AUDIO really is required here (unlike
    // CAMERA, which must not even be declared), so it is asked for at the
    // moment of use and the grant continues the action.
    val micPermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            viewModel.startRecording()
        } else {
            Toast.makeText(context, R.string.e_microphone_permission, Toast.LENGTH_LONG).show()
        }
    }
    val startRecording: () -> Unit = {
        val held = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.RECORD_AUDIO,
        ) == PackageManager.PERMISSION_GRANTED
        if (held) viewModel.startRecording() else micPermission.launch(Manifest.permission.RECORD_AUDIO)
    }

    // Placing a voice call: the same permission, asked the same way, and
    // the grant places the call. A refused start means this device is on
    // a call already (docs/protocol.md: one call per person).
    val placeCall: () -> Unit = {
        if (!viewModel.startCall()) {
            Toast.makeText(context, R.string.e_call_failed_to_start, Toast.LENGTH_SHORT).show()
        }
    }
    val callPermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            placeCall()
        } else {
            Toast.makeText(context, R.string.e_microphone_permission, Toast.LENGTH_LONG).show()
        }
    }
    val startCall: () -> Unit = {
        val held = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.RECORD_AUDIO,
        ) == PackageManager.PERMISSION_GRANTED
        if (held) placeCall() else callPermission.launch(Manifest.permission.RECORD_AUDIO)
    }

    // Sharing a location. Asked at the moment of use, and the grant
    // CONTINUES the action rather than making the person tap again — the
    // same rule the microphone follows. `RequestMultiplePermissions`
    // because either coarse or fine is enough: a member who allows only
    // approximate location still gets the feature.
    val locationPermission = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants ->
        if (grants.values.any { it }) {
            viewModel.shareLocation()
        } else {
            Toast.makeText(context, R.string.e_location_permission, Toast.LENGTH_LONG).show()
        }
    }
    val shareLocation: () -> Unit = {
        if (viewModel.hasLocationPermission()) {
            viewModel.shareLocation()
        } else {
            locationPermission.launch(LocationProvider.PERMISSIONS)
        }
    }

    // Saving to the gallery. On 26–28 the write needs a permission first;
    // `pendingSave` holds what the user asked for across that prompt so the
    // grant continues the action instead of dropping it.
    var pendingSave by remember { mutableStateOf<AttachmentDto?>(null) }
    val runSave: (AttachmentDto) -> Unit = { attachment ->
        scope.launch {
            when (viewModel.saveToGallery(context, attachment)) {
                GallerySaver.Result.SAVED ->
                    Toast.makeText(context, savedToGalleryLabel, Toast.LENGTH_SHORT).show()
                GallerySaver.Result.NEEDS_PERMISSION ->
                    viewModel.reportSaveNeedsPermission()
                GallerySaver.Result.FAILED -> Unit // the strip already says so
            }
        }
    }
    val requestStorage = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        val attachment = pendingSave
        pendingSave = null
        when {
            attachment == null -> Unit
            granted -> runSave(attachment)
            else -> viewModel.reportSaveNeedsPermission()
        }
    }
    val saveAttachment: (AttachmentDto) -> Unit = { attachment ->
        if (viewModel.savingNeedsPermission) {
            pendingSave = attachment
            requestStorage.launch(android.Manifest.permission.WRITE_EXTERNAL_STORAGE)
        } else {
            runSave(attachment)
        }
    }

    // Sharing an attachment: fetch the bytes if this device does not have
    // them, then hand the file to the chooser. A content:// Uri from
    // FileProvider, never file:// — and never the raw bitmap, which would
    // re-encode what the sender sent.
    val shareAttachment: (AttachmentDto, String) -> Unit = { attachment, caption ->
        scope.launch {
            viewModel.reportAttachmentBusy(preparingLabel)
            val file = viewModel.localFile(attachment)
            if (file == null) {
                viewModel.reportAttachmentOpenFailed(downloaded = false)
            } else {
                viewModel.clearMediaState()
                val shared = shareWithSystem(context, file, attachment.mime, caption)
                if (!shared) viewModel.reportAttachmentOpenFailed(downloaded = true)
            }
        }
    }

    // Downloading a file and handing it to whatever app can read it.
    // Failures land in the composer's strip, which is the one place this
    // screen already reports trouble with an attachment.
    val openFile: (AttachmentDto) -> Unit = { attachment ->
        scope.launch {
            // Say something immediately: a large PDF takes seconds, and a
            // tap that looks like nothing invites a second tap.
            viewModel.reportAttachmentBusy(preparingLabel)
            val file = viewModel.localFile(attachment)
            val opened = file != null && openWithSystem(context, file, attachment.mime)
            if (opened) {
                viewModel.clearMediaState()
            } else {
                viewModel.reportAttachmentOpenFailed(file != null)
            }
        }
    }

    val haptics = LocalHapticFeedback.current
    // Every path that actually applies a toggle (chip tap, capsule pick,
    // grid pick) funnels through here so the confirm tick fires exactly
    // when the reaction changes.
    val applyToggle: (Long, String) -> Unit = { serverId, emoji ->
        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
        viewModel.toggleReaction(serverId, emoji)
    }

    // The screen counts as "reading" only while RESUMED — this also
    // registers/clears the open chat for the unread-bump rule.
    LifecycleResumeEffect(Unit) {
        viewModel.setResumed(true)
        onPauseOrDispose { viewModel.setResumed(false) }
    }

    // The other half of "reading": whether the newest message is on
    // screen at all. Same index <= 1 test the follow rule below uses
    // (reverseLayout, so index 0 is the newest, at the bottom; 1 covers
    // the anchor holding onto the previous newest row). Only the
    // LazyListState knows this, so the screen has to hand it over —
    // without it the ViewModel reports a chat read wherever the list
    // happens to be parked, and the server's marker never comes back.
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex <= 1 }
            .distinctUntilChanged()
            .collect { viewModel.setAtNewest(it) }
    }

    // THE OPENING BRANCHES. Both of them end in setSettled(), which is
    // what lets the read collector believe the list's position — see
    // ChatViewModel's header for why an unsettled screen must never be
    // trusted to say "at the newest message".
    //
    // Gated on the ViewModel's own `settled` rather than on a
    // rememberSaveable: it survives a rotation exactly as one would, so
    // a reader who has already scrolled away is not re-anchored — and
    // unlike a saveable it cannot outlive the flow it gates, which is
    // what would leave a restored screen anchored and never settled,
    // i.e. never able to report a read at all.
    LaunchedEffect(openAnchor) {
        val decision = openAnchor ?: return@LaunchedEffect
        if (settled) return@LaunchedEffect
        when (decision) {
            // Nothing unread, or the anchor gave up: the chat opens
            // where it always did, and there is nothing to wait for.
            is OpenAnchor.Newest -> viewModel.setSettled()
            // Anchored: hand the target to the same pipeline a quote tap
            // uses. Whichever way that ends — found and scrolled, or out
            // of pages — it settles.
            is OpenAnchor.Message ->
                pendingJump = JumpRequest(serverId = decision.serverId, anchor = true)
        }
    }

    // Jump to a quoted message, or to the opening anchor. Best-effort by
    // design: the thread holds a bounded window, so this pages older a
    // few times looking for the target and then gives up — for a quote
    // the excerpt is already on screen, which is most of what the tap was
    // asking for. Pages OLDER only: both kinds of target point backwards.
    // Searching for the target: pages older, bounded, then gives up. It
    // deliberately does NOT scroll or highlight — writing pendingJump
    // from inside an effect keyed on pendingJump cancels that same
    // effect, which is what silently killed the scroll and the highlight
    // mid-flight. Finding it hands over to the effect below.
    LaunchedEffect(pendingJump, items) {
        val request = pendingJump ?: return@LaunchedEffect
        val found = items.any {
            it is ChatListItem.MessageItem && it.entity.serverId == request.serverId
        }
        when {
            found -> {
                jumpPagesTried = 0
                pendingJump = null
                scrollToJump = request
            }
            // A page is already in flight; this effect re-runs when it lands.
            loadingOlder -> Unit
            // Bounded on its OWN count, never on the window growing:
            // loadOlder widens the window whether or not the network
            // answered, so an offline search would otherwise appear to
            // make progress forever while fetching nothing. Without a cap
            // this also re-fires on every later change to `items` (a new
            // message arriving is enough) and would keep paging a history
            // that does not contain the target — the normal case for a
            // quote once a chat is old enough.
            items.isNotEmpty() && jumpPagesTried < ChatViewModel.MAX_JUMP_PAGES -> {
                jumpPagesTried++
                viewModel.loadOlder()
            }
            else -> {
                pendingJump = null
                jumpPagesTried = 0
                // Out of pages. For an anchored open that is the
                // give-up: the reader stays at the newest message, and
                // the screen has to settle or it would never report a
                // read again.
                if (request.anchor) viewModel.setSettled()
            }
        }
    }

    // Doing the move, keyed only on the request it is moving to, so
    // nothing it writes can restart it. `items` is read but not a key: a
    // message arriving mid-scroll must not cancel the highlight, and the
    // index is recomputed inside anyway.
    LaunchedEffect(scrollToJump) {
        val request = scrollToJump ?: return@LaunchedEffect
        if (request.anchor) {
            // The offset comes from the LIST's viewport, not from the
            // configuration's screen height — the latter counts neither
            // the IME nor the composer, and in a reverseLayout list an
            // offset that is too small lands the anchor at the BOTTOM
            // edge with every unread message off screen ABOVE it, which
            // looks exactly like the bug this feature exists to fix. The
            // viewport is only known once the list has laid out, so wait
            // for it rather than reading a zero on the first frame.
            // Bounded, because the ONE thing this must never do is fail
            // to finish: settling is what re-arms read reporting, and a
            // screen that waits forever for a viewport it is never going
            // to get would stop reporting reads for as long as it is
            // open. Timing out means opening where we already are, which
            // is the same answer the search's give-up gives.
            val viewportPx = withTimeoutOrNull(ANCHOR_VIEWPORT_TIMEOUT_MS) {
                snapshotFlow { listState.layoutInfo.viewportSize.height }.first { it > 0 }
            }
            if (viewportPx != null) {
                val offset = -(viewportPx - anchorTopMarginPx).coerceAtLeast(0)
                // Twice, recomputing between: item indices are not
                // message indices (date pills are interleaved), the
                // divider row is built from the same state change that
                // started this and may not be in `items` on the first
                // pass, and a message arriving mid-move shifts
                // everything by one. The second pass costs nothing when
                // the first one already landed.
                repeat(2) { pass ->
                    val index = anchorIndex(items, request.serverId)
                    if (index >= 0) listState.scrollToItem(index, scrollOffset = offset)
                    if (pass == 0) delay(ANCHOR_SETTLE_MS)
                }
            }
            // Settled BEFORE the key is cleared: this is the line that
            // re-arms read reporting, and it must not depend on how
            // Compose schedules the restart of an effect that just
            // rewrote its own key.
            viewModel.setSettled()
            scrollToJump = null
            return@LaunchedEffect
        }
        val index = items.indexOfFirst {
            it is ChatListItem.MessageItem && it.entity.serverId == request.serverId
        }
        if (index >= 0) {
            // Centred, like iOS: the quoted message with its neighbourhood
            // still visible, rather than jammed against the bottom edge.
            listState.animateScrollToItem(index, scrollOffset = -centreOffsetPx)
            highlightedMessageId = request.serverId
            delay(HIGHLIGHT_MS)
            if (highlightedMessageId == request.serverId) highlightedMessageId = null
        }
        scrollToJump = null
    }

    // guarded loadOlder. derivedStateOf collapses scroll churn into a
    // boolean edge; the effect only runs on the flip to true. The
    // items.isNotEmpty() guard lives INSIDE the derivation (items is
    // snapshot state, so it stays reactive): the chat-empty item alone
    // must not count as "near the old end", and guarding in the effect
    // body instead would latch nearOldEnd true during the empty state
    // and never re-flip once messages arrive.
    val nearOldEnd by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            items.isNotEmpty() && info.totalItemsCount > 0 && last >= info.totalItemsCount - 10
        }
    }
    // Suppressed until the screen has settled. An anchored open parks
    // the reader well up the list, which is inside this trigger's band —
    // so it would fire a page fetch on open, in a race with the anchor
    // search's own bounded paging, for history nobody has scrolled to
    // yet. The anchor search calls loadOlder directly for what it needs;
    // once the screen settles this takes over as usual.
    LaunchedEffect(nearOldEnd, settled) {
        if (nearOldEnd && settled) viewModel.loadOlder()
    }

    // Follow new messages explicitly. The LazyColumn only auto-follows
    // an index-0 insert while resting at EXACTLY offset zero — a few
    // pixels of drift (keyboard churn, a hair of overscroll) and a
    // fresh message stays hidden below the fold. Same rules as iOS: my
    // own send always lands the list on the new bubble, an inbound
    // message follows only when the user is already at the bottom
    // (index ≤ 1 covers "the anchor held onto the previous newest
    // row"); a reader deep in history is never yanked down.
    val newestMessage by remember {
        derivedStateOf {
            items.firstOrNull { it is ChatListItem.MessageItem } as? ChatListItem.MessageItem
        }
    }
    var lastNewestKey by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(newestMessage?.key) {
        val newest = newestMessage ?: return@LaunchedEffect
        val previousKey = lastNewestKey
        lastNewestKey = newest.key
        // First emission = the chat opening at the bottom, not a new
        // message; a same-key relaunch is just this effect settling.
        if (previousKey == null || previousKey == newest.key) return@LaunchedEffect
        // Not while the screen is still opening. index <= 1 is true of
        // every list that has not been scrolled yet, anchored or not, so
        // a message arriving mid-open would pull the reader to the
        // bottom out from under the anchor that is about to move them.
        if (!settled) return@LaunchedEffect
        if (newest.entity.senderId == myUserId || listState.firstVisibleItemIndex <= 1) {
            listState.animateScrollToItem(0)
        }
    }

    // The bar tonally lifts once messages scroll beneath it. Driven
    // from the list, not pinnedScrollBehavior: reverseLayout inverts
    // the nestedScroll deltas the behavior reads (and old-end overscroll
    // can latch the bar lifted), while canScrollForward is true exactly
    // while older messages extend beneath the bar.
    val barLifted by remember { derivedStateOf { listState.canScrollForward } }
    val barColor by animateColorAsState(
        targetValue = if (barLifted) {
            MaterialTheme.colorScheme.surfaceContainer
        } else {
            MaterialTheme.colorScheme.surface
        },
        label = "chatBarLift",
    )

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = chat?.title ?: "",
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        // The subtitle line is reserved even while nobody is
                        // typing, so the bar never changes height. Sized from
                        // the label's own line height (sp → dp), not a fixed
                        // dp, so large font scales don't clip the text.
                        val subtitleHeight = with(LocalDensity.current) {
                            MaterialTheme.typography.labelSmall.lineHeight.toDp()
                        }
                        Box(modifier = Modifier.height(subtitleHeight)) {
                            // Keep the last name around for the fade-out —
                            // typingUser is already null during the exit.
                            val lastTypingUser = remember { mutableStateOf("") }
                            typingUser?.let { lastTypingUser.value = it }
                            // Fully qualified: picks the top-level overload —
                            // the enclosing bar Column's ColumnScope extension
                            // is not callable implicitly from this Box.
                            androidx.compose.animation.AnimatedVisibility(
                                visible = typingUser != null,
                                enter = fadeIn(tween(150)),
                                exit = fadeOut(tween(150)),
                            ) {
                                Text(
                                    text = stringResource(R.string.s_is_typing, lastTypingUser.value),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.primary,
                                    maxLines = 1,
                                )
                            }
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.s_back))
                    }
                },
                actions = {
                    // A direct chat is the one place a call can start from:
                    // a call lives in a direct chat (docs/protocol.md,
                    // "Voice calls"). Hidden, not disabled, on a server
                    // that has calls off.
                    if (callsEnabled && chat?.kind == "direct") {
                        IconButton(onClick = startCall) {
                            Icon(Icons.Filled.Call, contentDescription = stringResource(R.string.s_voice_call))
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = barColor),
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
                .navigationBarsPadding(),
        ) {
            OfflineBanner(isOnline = isOnline, socketState = socketState)

            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
            ) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    reverseLayout = true,
                    contentPadding = PaddingValues(
                        horizontal = 12.dp,
                        vertical = 8.dp,
                    ),
                ) {
                    if (items.isEmpty() && initialLoadSettled) {
                        item(key = "chat-empty") {
                            Box(
                                modifier = Modifier.fillParentMaxSize(),
                                contentAlignment = Alignment.Center,
                            ) {
                                EmptyState(
                                    icon = Icons.Outlined.Forum,
                                    title = stringResource(R.string.s_no_messages_yet),
                                    // kind is "family" | "direct" (ChatEntity)
                                    // — a 1:1 chat is not "your family chat".
                                    subtitle = stringResource(
                                        if (chat?.kind == "direct") {
                                            R.string.s_say_hi_direct
                                        } else {
                                            R.string.s_say_hi_family
                                        },
                                    ),
                                )
                            }
                        }
                    }
                    items(items, key = { it.key }) { item ->
                        // A jumped-to bubble is briefly tinted, so the eye
                        // lands on the right one in a wall of text.
                        val jumped = item is ChatListItem.MessageItem &&
                            item.entity.serverId != null &&
                            item.entity.serverId == highlightedMessageId
                        val highlight by animateColorAsState(
                            targetValue = if (jumped) {
                                MaterialTheme.colorScheme.primary.copy(alpha = 0.12f)
                            } else {
                                Color.Transparent
                            },
                            animationSpec = tween(250),
                            label = "jumpHighlight",
                        )
                        Box(
                            modifier = Modifier
                                .animateItem(
                                    fadeInSpec = tween(200),
                                    placementSpec = spring(stiffness = Spring.StiffnessMediumLow),
                                )
                                .background(highlight, RoundedCornerShape(12.dp)),
                        ) {
                            when (item) {
                                is ChatListItem.DateSeparator -> DateSeparatorPill(item.label)
                                is ChatListItem.NewMessagesDivider ->
                                    NewMessagesDividerRow(item.count)
                                is ChatListItem.MessageItem -> MessageBubble(
                                    item = item,
                                    chat = chat,
                                    isMine = item.entity.senderId == myUserId,
                                    isStreaming = item.entity.serverId
                                        ?.let { it in streamingIds } == true,
                                    myUserId = myUserId,
                                    memberNames = memberNames,
                                    memberAvatars = memberAvatars,
                                    linkPreviews = linkPreviews,
                                    previewsEnabled = linkPreviewsEnabled,
                                    mapPreviewsEnabled = mapPreviewsEnabled,
                                    onRequestPreview = viewModel::requestLinkPreview,
                                    streamUrl = viewModel::attachmentStreamUrl,
                                    onFailedTap = { failedActionTarget = it },
                                    onToggleReaction = applyToggle,
                                    onVote = { serverId, optionId ->
                                        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                                        viewModel.vote(serverId, optionId)
                                    },
                                    onCallBack = if (callsEnabled && chat?.kind == "direct") startCall else null,
                                    onTapQuote = {
                                        pendingJump = JumpRequest(serverId = it, anchor = false)
                                    },
                                    onOpenAttachment = { attachment ->
                                        if (attachment.isFile) {
                                            openFile(attachment)
                                        } else {
                                            viewingAttachment = attachment
                                        }
                                    },
                                    onLongPress = { pressed, bounds ->
                                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                        pickerTarget = ReactionPickerTarget(pressed, bounds)
                                    },
                                    // While the capsule is open its anchor
                                    // tracks the bubble's live bounds — the
                                    // focusable popup closes the keyboard and
                                    // the list shifts under it, so a one-shot
                                    // snapshot would go stale. No-op (no
                                    // state write) while no capsule is open.
                                    onPositioned = { positioned, rect ->
                                        val current = pickerTarget
                                        if (current != null &&
                                            current.item.key == positioned.key &&
                                            current.anchorBounds != rect
                                        ) {
                                            pickerTarget = current.copy(anchorBounds = rect)
                                        }
                                    },
                                )
                            }
                        }
                    }
                    if (items.isNotEmpty()) {
                        // Last index = the visually-top (oldest) end. The 32dp
                        // box is always composed so the item's height truly
                        // never changes; only the spinner inside fades, so
                        // appearing never shifts the oldest messages and the
                        // exit animation can actually play. Gated on a
                        // non-empty list so this extra item never renders in
                        // an empty chat; the chat-empty item is likewise
                        // excluded from the nearOldEnd trigger by its
                        // items.isNotEmpty() guard.
                        item(key = "chat-older-loading") {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(32.dp),
                                contentAlignment = Alignment.Center,
                            ) {
                                androidx.compose.animation.AnimatedVisibility(
                                    visible = loadingOlder,
                                    enter = fadeIn(tween(150)),
                                    exit = fadeOut(tween(150)),
                                ) {
                                    CircularProgressIndicator(
                                        modifier = Modifier.size(20.dp),
                                        strokeWidth = 2.dp,
                                    )
                                }
                            }
                        }
                    }
                }

                // Scroll-to-newest: a pure overlay above the input bar; in
                // this reverseLayout list index 0 is the newest message.
                // The decision is showScrollToNewest's (see its header):
                // settled AND meaningfully off the bottom — the read
                // gate's own at-newest shape. The old `index > 5` hid
                // the button behind several screens of tall bubbles, and
                // an unsettled screen must never flash it mid-open.
                val showScrollToBottom by remember(settled) {
                    derivedStateOf {
                        showScrollToNewest(
                            settled = settled,
                            firstVisibleItemIndex = listState.firstVisibleItemIndex,
                        )
                    }
                }
                androidx.compose.animation.AnimatedVisibility(
                    visible = showScrollToBottom,
                    enter = scaleIn() + fadeIn(),
                    exit = scaleOut() + fadeOut(),
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(16.dp),
                ) {
                    SmallFloatingActionButton(
                        onClick = { scope.launch { listState.animateScrollToItem(0) } },
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        contentColor = MaterialTheme.colorScheme.primary,
                    ) {
                        Icon(
                            imageVector = Icons.Filled.KeyboardArrowDown,
                            contentDescription = stringResource(R.string.s_scroll_to_newest),
                        )
                    }
                }
            }

            viewingAttachment?.let { attachment ->
                AttachmentViewer(
                    attachment = attachment,
                    streamUrl = viewModel::attachmentStreamUrl,
                    onShare = {
                        viewingAttachment = null
                        shareAttachment(attachment, "")
                    },
                    onSave = {
                        viewingAttachment = null
                        saveAttachment(attachment)
                    },
                    onDismiss = { viewingAttachment = null },
                )
            }
            InputBar(
                state = viewModel.inputState,
                onSend = viewModel::send,
                replyDraft = replyDraft,
                replyAuthorName = replyDraft?.senderId?.let { sender ->
                    if (sender == myUserId) {
                        stringResource(R.string.s_you)
                    } else {
                        memberNames[sender] ?: stringResource(R.string.s_someone)
                    }
                } ?: "",
                onCancelReply = viewModel::cancelReply,
                focusRequester = focusRequester,
                isEditing = editTarget != null,
                onCancelEdit = viewModel::cancelEdit,
                mediaState = mediaState,
                onPickMedia = {
                    pickMedia.launch(
                        PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo),
                    )
                },
                onPickFile = { pickFile.launch(arrayOf("*/*")) },
                onPasteFromClipboard = pasteFromClipboard,
                onPasteContent = pasteContent,
                onPasteTruncated = viewModel::reportPasteTruncated,
                showsPoll = canCreatePoll,
                onStartPoll = viewModel::beginPoll,
                staged = staged,
                onTakePhoto = { startCapture(false) },
                onTakeVideo = { startCapture(true) },
                onRecordAudio = startRecording,
                recordingMs = recordingMs,
                onStopRecording = viewModel::stopRecording,
                onCancelRecording = viewModel::cancelRecording,
                onDiscardStaged = viewModel::discardStaged,
                onDismissMediaError = viewModel::clearMediaState,
                showsAssistantMention = chat?.kind == "family" && assistantUserId != null,
                onShareLocation = shareLocation,
            )
        }
    }

    pickerTarget?.let { target ->
        // The popup defers every close until its exit animations settle
        // (see ReactionPickerPopup) — so onPick only applies the toggle
        // (instantly), and the actual removal always arrives via
        // onDismiss / onMore afterwards.
        ReactionPickerPopup(
            target = target,
            myUserId = myUserId,
            onPick = { emoji ->
                target.item.entity.serverId?.let { applyToggle(it, emoji) }
            },
            onMore = {
                pickerTarget = null
                fullPickerTarget = target.item
            },
            onEdit = {
                val entity = target.item.entity
                pickerTarget = null
                entity.serverId?.let { serverId ->
                    viewModel.beginEdit(serverId, entity.body)
                    focusRequester.requestFocus()
                }
            },
            onReply = {
                val entity = target.item.entity
                pickerTarget = null
                entity.serverId?.let { serverId ->
                    viewModel.beginReply(
                        ReplyToDto(
                            messageId = serverId,
                            senderId = entity.senderId,
                            // Cut exactly as the server will, so the
                            // banner and the final bubble agree.
                            excerpt = ReplyToDto.excerpt(entity.body),
                        ),
                    )
                    focusRequester.requestFocus()
                }
            },
            onClosePoll = {
                val entity = target.item.entity
                pickerTarget = null
                entity.serverId?.let { viewModel.closePoll(it) }
            },
            onCopy = {
                val body = target.item.entity.body
                pickerTarget = null
                // setClipEntry is suspend, hence the scope.
                scope.launch {
                    clipboard.setClipEntry(
                        ClipData.newPlainText(clipLabel, body).toClipEntry(),
                    )
                    // Android 13+ shows its own copy confirmation; ours
                    // on top of it would be a second one. A toast rather
                    // than a snackbar because this screen's bottom edge
                    // is the input bar, which a snackbar would cover.
                    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                        Toast.makeText(context, copiedLabel, Toast.LENGTH_SHORT).show()
                    }
                }
            },
            onSave = {
                val attachment = target.item.entity.attachment
                pickerTarget = null
                if (attachment != null) saveAttachment(attachment)
            },
            onShare = {
                val entity = target.item.entity
                val body = entity.body
                val attachment = entity.attachment
                pickerTarget = null
                if (attachment != null) {
                    shareAttachment(attachment, body)
                } else {
                    val send = Intent(Intent.ACTION_SEND).apply {
                        type = "text/plain"
                        putExtra(Intent.EXTRA_TEXT, body)
                    }
                    context.startActivity(Intent.createChooser(send, null))
                }
            },
            onDismiss = { pickerTarget = null },
        )
    }

    fullPickerTarget?.let { target ->
        ModalBottomSheet(
            onDismissRequest = { fullPickerTarget = null },
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
        ) {
            Text(
                text = stringResource(R.string.s_react),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
            )
            EmojiCatalogGrid(
                onPick = { emoji ->
                    target.entity.serverId?.let { applyToggle(it, emoji) }
                    fullPickerTarget = null
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f, fill = false),
            )
        }
    }

    pollDraft?.let { draft ->
        PollComposerSheet(
            draft = draft,
            onQuestionChange = viewModel::setPollQuestion,
            onOptionChange = viewModel::setPollOption,
            onAddOption = viewModel::addPollOption,
            onRemoveOption = viewModel::removePollOption,
            onSend = viewModel::sendPoll,
            onDismiss = viewModel::cancelPoll,
        )
    }

    failedActionTarget?.let { clientMsgId ->
        AlertDialog(
            onDismissRequest = { failedActionTarget = null },
            title = { Text(stringResource(R.string.s_message_not_sent)) },
            text = { Text(stringResource(R.string.s_try_sending_it_again_or_delete_the_draft)) },
            confirmButton = {
                TextButton(onClick = {
                    viewModel.retry(clientMsgId)
                    failedActionTarget = null
                }) {
                    Text(stringResource(R.string.s_retry))
                }
            },
            dismissButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_delete),
                    onClick = {
                        viewModel.deleteFailed(clientMsgId)
                        failedActionTarget = null
                    },
                )
            },
        )
    }
}

/**
 * Full-window Popup hosting the scrim and the floating reaction capsule.
 * The position provider pins the popup to the window origin, so the
 * capsule can be absolutely placed against the bubble's boundsInWindow:
 * horizontally centered on the bubble, directly above it — or below when
 * the bubble sits too near the top — and clamped to the window with a
 * margin. Back press, scrim tap, and picking all dismiss — deferred:
 * each close path flips the shared transition off via exitThen and only
 * runs its action once the exit animations have settled, because
 * clearing pickerTarget immediately would unmount the Popup and the
 * declared exits could never play.
 */
@Composable
private fun ReactionPickerPopup(
    target: ReactionPickerTarget,
    myUserId: Long?,
    onPick: (String) -> Unit,
    onMore: () -> Unit,
    onReply: () -> Unit,
    onEdit: () -> Unit,
    onClosePoll: () -> Unit,
    onCopy: () -> Unit,
    onShare: () -> Unit,
    onSave: () -> Unit,
    onDismiss: () -> Unit,
) {
    val atWindowOrigin = remember {
        object : PopupPositionProvider {
            override fun calculatePosition(
                anchorBounds: IntRect,
                windowSize: IntSize,
                layoutDirection: LayoutDirection,
                popupContentSize: IntSize,
            ): IntOffset = IntOffset.Zero
        }
    }
    // Shared entrance/exit transition: the scrim fades, the capsule
    // scales+fades against the bubble's edge. Hoisted above the Popup so
    // onDismissRequest can route through exitThen too.
    val entrance = remember { MutableTransitionState(false).apply { targetState = true } }
    var pending by remember { mutableStateOf<(() -> Unit)?>(null) }
    // Play the exit, then run the close action; the first caller wins so
    // a scrim tap during an in-flight exit can't re-arm a second action.
    fun exitThen(action: () -> Unit) {
        if (pending == null) {
            pending = action
            entrance.targetState = false
        }
    }
    LaunchedEffect(entrance) {
        snapshotFlow { entrance.isIdle && !entrance.currentState }
            .collect { exited -> if (exited) pending?.invoke() }
    }
    Popup(
        popupPositionProvider = atWindowOrigin,
        onDismissRequest = { exitThen(onDismiss) },
        properties = PopupProperties(focusable = true),
    ) {
        var containerSize by remember { mutableStateOf(IntSize.Zero) }
        var capsuleSize by remember { mutableStateOf(IntSize.Zero) }
        var menuSize by remember { mutableStateOf(IntSize.Zero) }
        val density = LocalDensity.current
        val margin = with(density) { 12.dp.roundToPx() }
        val gap = with(density) { 8.dp.roundToPx() }

        val anchor = target.anchorBounds
        val showBelow = anchor.top - gap - capsuleSize.height < margin
        val x = (anchor.center.x - capsuleSize.width / 2f).roundToInt()
            .coerceIn(margin, maxOf(margin, containerSize.width - capsuleSize.width - margin))
        val y = if (showBelow) {
            (anchor.bottom + gap).roundToInt()
                .coerceAtMost(maxOf(margin, containerSize.height - capsuleSize.height - margin))
        } else {
            anchor.top.roundToInt() - gap - capsuleSize.height
        }
        // The menu goes under the bubble, but never on top of the
        // capsule: a bubble taller than the window pins BOTH against the
        // same edge, and the menu is drawn second, so without this it
        // would simply cover the capsule and take the reactions with it.
        // Stack under the capsule when there is room, otherwise above.
        val menuX = (anchor.center.x - menuSize.width / 2f).roundToInt()
            .coerceIn(margin, maxOf(margin, containerSize.width - menuSize.width - margin))
        val preferredMenuY = (anchor.bottom.roundToInt() + gap)
            .coerceIn(margin, maxOf(margin, containerSize.height - menuSize.height - margin))
        val capsuleBottom = y + capsuleSize.height
        val overlapsCapsule = preferredMenuY < capsuleBottom + gap &&
            preferredMenuY + menuSize.height > y - gap
        val menuY = when {
            !overlapsCapsule -> preferredMenuY
            capsuleBottom + gap + menuSize.height <= containerSize.height - margin ->
                capsuleBottom + gap
            else -> maxOf(margin, y - gap - menuSize.height)
        }

        Box(
            modifier = Modifier
                .fillMaxSize()
                .onSizeChanged { containerSize = it },
        ) {
            AnimatedVisibility(visibleState = entrance, enter = fadeIn(), exit = fadeOut()) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.32f))
                        .clickable(
                            interactionSource = remember { MutableInteractionSource() },
                            indication = null,
                            onClick = { exitThen(onDismiss) },
                        ),
                )
            }
            AnimatedVisibility(
                visibleState = entrance,
                enter = fadeIn() + scaleIn(
                    initialScale = 0.8f,
                    transformOrigin = TransformOrigin(0.5f, if (showBelow) 0f else 1f),
                ),
                exit = fadeOut() + scaleOut(),
                modifier = Modifier.offset { IntOffset(x, y) },
            ) {
                ReactionCapsule(
                    myReaction = target.item.myReaction,
                    onPick = { emoji ->
                        // Apply the toggle immediately — the optimistic chip
                        // update must not wait for the capsule's exit — and
                        // defer only the popup removal.
                        onPick(emoji)
                        exitThen(onDismiss)
                    },
                    onMore = { exitThen(onMore) },
                    modifier = Modifier.onSizeChanged { capsuleSize = it },
                )
            }
            AnimatedVisibility(
                visibleState = entrance,
                enter = fadeIn() + scaleIn(
                    initialScale = 0.8f,
                    transformOrigin = TransformOrigin(0.5f, 0f),
                ),
                exit = fadeOut() + scaleOut(),
                modifier = Modifier.offset { IntOffset(menuX, menuY) },
            ) {
                MessageContextMenu(
                    onReply = { exitThen(onReply) },
                    onEdit = { exitThen(onEdit) },
                    onClosePoll = { exitThen(onClosePoll) },
                    onCopy = { exitThen(onCopy) },
                    onShare = { exitThen(onShare) },
                    onSave = { exitThen(onSave) },
                    modifier = Modifier.onSizeChanged { menuSize = it },
                    canSave = target.item.entity.attachment?.isFile == false,
                    canReply = target.item.entity.serverId != null,
                    canEdit = target.item.entity.serverId != null &&
                        target.item.entity.senderId == myUserId,
                    // Closing is the AUTHOR's, and one-way — the family
                    // owner does not outrank them here, exactly as with
                    // editing. Hidden once the poll is already closed:
                    // the server would no-op it, and an action that does
                    // nothing is worse than no action.
                    canClosePoll = target.item.poll?.closed == false &&
                        target.item.entity.serverId != null &&
                        target.item.entity.senderId == myUserId,
                    canCopy = target.item.entity.body.isNotEmpty(),
                )
            }
        }
    }
}

/**
 * The actions under the bubble, beside the reaction capsule above it.
 * Copy and share are both purely local — the message text is already in
 * hand — so neither waits on the network or on the popup's exit.
 */
/**
 * "Replying to X" above the input field, with the way out. Its appearance
 * changes the input bar's height — which is fine here, unlike on iOS,
 * because the thread is a reverseLayout list anchored at the newest
 * message.
 */
@Composable
private fun ReplyBanner(
    authorName: String,
    excerpt: String,
    onCancel: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 12.dp, end = 4.dp, top = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier
                .width(3.dp)
                .height(32.dp)
                .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(1.5.dp)),
        )
        Spacer(Modifier.width(8.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = "Replying to $authorName",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                text = excerpt,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        IconButton(onClick = onCancel) {
            Icon(
                imageVector = Icons.Filled.Close,
                contentDescription = stringResource(R.string.s_cancel_reply),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * stringResource(R.string.s_editing_message) above the input field, with the way out. Cancelling
 * puts the displaced draft back — the composer was borrowed, and giving it
 * back unchanged is the least surprising thing it can do.
 */
@Composable
private fun EditBanner(onCancel: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 12.dp, end = 4.dp, top = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Outlined.Edit,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(16.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            text = stringResource(R.string.s_editing_message),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = onCancel) {
            Icon(
                imageVector = Icons.Filled.Close,
                contentDescription = stringResource(R.string.s_cancel_editing),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun MessageContextMenu(
    onReply: () -> Unit,
    onEdit: () -> Unit,
    onClosePoll: () -> Unit,
    onCopy: () -> Unit,
    onShare: () -> Unit,
    onSave: () -> Unit,
    modifier: Modifier = Modifier,
    /**
     * Reply needs a server id to quote, so it is hidden — not disabled —
     * on a message that has not been acked yet.
     */
    canReply: Boolean = true,
    /** Only the author may edit, and only once the message has an id. */
    canEdit: Boolean = false,
    /** Only the author may close their poll, and only while it is open. */
    canClosePoll: Boolean = false,
    /** A photo sent without a caption has nothing to copy. */
    canCopy: Boolean = true,
    /** Photos and videos only — what the gallery will take. */
    canSave: Boolean = false,
) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 6.dp,
        modifier = modifier.width(IntrinsicSize.Max),
    ) {
        Column(modifier = Modifier.padding(vertical = 4.dp)) {
            if (canReply) {
                MessageContextMenuItem(
                    label = stringResource(R.string.s_reply),
                    icon = Icons.AutoMirrored.Outlined.Reply,
                    onClick = onReply,
                )
            }
            if (canEdit) {
                MessageContextMenuItem(
                    label = stringResource(R.string.s_edit),
                    icon = Icons.Outlined.Edit,
                    onClick = onEdit,
                )
            }
            if (canClosePoll) {
                MessageContextMenuItem(
                    label = stringResource(R.string.s_close_poll),
                    icon = Icons.Outlined.HowToVote,
                    onClick = onClosePoll,
                )
            }
            if (canCopy) {
                MessageContextMenuItem(
                    label = stringResource(R.string.s_copy),
                    icon = Icons.Outlined.ContentCopy,
                    onClick = onCopy,
                )
            }
            if (canSave) {
                // Android's chooser cannot do this on its own, unlike
                // iOS's share sheet — hence a row of its own here.
                MessageContextMenuItem(
                    label = stringResource(R.string.s_save_to_gallery),
                    icon = Icons.Outlined.Download,
                    onClick = onSave,
                )
            }
            MessageContextMenuItem(
                label = stringResource(R.string.s_share),
                icon = Icons.Outlined.Share,
                onClick = onShare,
            )
        }
    }
}

@Composable
private fun MessageContextMenuItem(
    label: String,
    icon: ImageVector,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp))
        Text(text = label, style = MaterialTheme.typography.bodyLarge)
    }
}

/**
 * The capsule itself: the quick set, plus my current reaction as an
 * extra trailing item when it is not in the quick set (so it can always
 * be seen and un-toggled), plus the "+" into the full picker. The row
 * scrolls horizontally as a safety valve on very narrow screens.
 */
@Composable
private fun ReactionCapsule(
    myReaction: String?,
    onPick: (String) -> Unit,
    onMore: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val emojis = if (myReaction != null && myReaction !in QUICK_REACTIONS) {
        QUICK_REACTIONS + myReaction
    } else {
        QUICK_REACTIONS
    }
    Surface(
        shape = RoundedCornerShape(28.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 3.dp,
        shadowElevation = 6.dp,
        modifier = modifier,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(2.dp),
            modifier = Modifier
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 6.dp),
        ) {
            emojis.forEach { emoji ->
                // key so each cell's press state stays with its emoji when
                // the trailing off-list reaction slides in and out.
                key(emoji) {
                    val selected = emoji == myReaction
                    val pressSource = remember { MutableInteractionSource() }
                    val pressScale = emojiPressScale(pressSource)
                    Box(
                        contentAlignment = Alignment.Center,
                        modifier = Modifier
                            .graphicsLayer {
                                scaleX = pressScale.value
                                scaleY = pressScale.value
                            }
                            .clip(CircleShape)
                            .background(
                                if (selected) {
                                    MaterialTheme.colorScheme.primaryContainer
                                } else {
                                    Color.Transparent
                                },
                            )
                            .clickable(
                                interactionSource = pressSource,
                                indication = ripple(),
                            ) { onPick(emoji) }
                            .padding(5.dp),
                    ) {
                        Text(emoji, style = MaterialTheme.typography.titleLarge)
                    }
                }
            }
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .clickable(onClick = onMore)
                    .padding(6.dp),
            ) {
                Icon(
                    imageVector = Icons.Filled.Add,
                    contentDescription = stringResource(R.string.s_more_reactions),
                    modifier = Modifier.size(22.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * The categorized full-picker grid behind the capsule's "+": full-span
 * section headers over an adaptive emoji grid, straight from
 * EMOJI_CATALOG (kept byte-identical with iOS — see EmojiCatalog.kt).
 */
@Composable
private fun EmojiCatalogGrid(
    onPick: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(minSize = 48.dp),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
        modifier = modifier,
    ) {
        EMOJI_CATALOG.forEach { category ->
            item(key = "header:${category.name}", span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    text = category.name,
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 12.dp, bottom = 4.dp),
                )
            }
            items(category.emoji, key = { "${category.name}:$it" }) { emoji ->
                val pressSource = remember { MutableInteractionSource() }
                val pressScale = emojiPressScale(pressSource)
                Box(
                    contentAlignment = Alignment.Center,
                    modifier = Modifier
                        .aspectRatio(1f)
                        .graphicsLayer {
                            scaleX = pressScale.value
                            scaleY = pressScale.value
                        }
                        .clip(RoundedCornerShape(8.dp))
                        .clickable(
                            interactionSource = pressSource,
                            indication = ripple(),
                        ) { onPick(emoji) },
                ) {
                    Text(emoji, style = MaterialTheme.typography.headlineSmall)
                }
            }
        }
    }
}

/**
 * Shared press feedback for emoji cells (capsule + full grid): a bouncy
 * scale to apply from graphicsLayer while pressed. Returned as State so
 * the value is read in the draw phase only — the pop is draw-only and
 * never shifts neighboring cells.
 */
@Composable
private fun emojiPressScale(interactionSource: MutableInteractionSource): State<Float> {
    val pressed by interactionSource.collectIsPressedAsState()
    return animateFloatAsState(
        targetValue = if (pressed) 1.2f else 1f,
        animationSpec = spring(dampingRatio = Spring.DampingRatioMediumBouncy),
        label = "emojiPressScale",
    )
}

@Composable
private fun DateSeparatorPill(label: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        // Outlined lowest-tone pill so the separator reads as chrome, not
        // as another message bubble.
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainerLowest,
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
            )
        }
    }
}

/**
 * "N new messages" — the line the reader started from.
 *
 * A rule with the count in the middle, in the primary tint so it reads
 * as a mark rather than as chrome (the day pill is deliberately the
 * quieter of the two, and they can sit one above the other). Full width
 * and centred, so it works over either side's bubbles.
 *
 * The count is the chat's unread count as it stood at OPEN and does not
 * tick down while the reader reads — see ChatListItem.NewMessagesDivider.
 *
 * iOS counterpart: the same row in ConversationView's day-section model.
 */
@Composable
private fun NewMessagesDividerRow(count: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val line = MaterialTheme.colorScheme.primary.copy(alpha = 0.4f)
        HorizontalDivider(modifier = Modifier.weight(1f), color = line)
        Text(
            text = pluralStringResource(R.plurals.s_n_new_messages, count, count),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        HorizontalDivider(modifier = Modifier.weight(1f), color = line)
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageBubble(
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    /** The assistant is still writing into this row. */
    isStreaming: Boolean,
    myUserId: Long?,
    memberNames: Map<Long, String>,
    memberAvatars: Map<Long, Long>,
    linkPreviews: Map<String, LinkPreviewState>,
    previewsEnabled: Boolean,
    /** Whether a shared location draws a map — the reader's own setting. */
    mapPreviewsEnabled: Boolean,
    onRequestPreview: (String) -> Unit,
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onFailedTap: (String) -> Unit,
    onToggleReaction: (Long, String) -> Unit,
    /** (message server id, option id) — a tap on a poll option. */
    onVote: (Long, Long) -> Unit,
    onLongPress: (ChatListItem.MessageItem, Rect) -> Unit,
    onPositioned: (ChatListItem.MessageItem, Rect) -> Unit,
    onTapQuote: (Long) -> Unit,
    onOpenAttachment: (AttachmentDto) -> Unit,
    /** Tapping a call record calls back; null where calling is not possible. */
    onCallBack: (() -> Unit)? = null,
) {
    val entity = item.entity
    // 18dp corners, tightened to 4dp where a bubble meets a same-sender
    // run mate on the sender's side (the Google Messages idiom). In this
    // reverseLayout list the OLDER neighbor renders above, so isRunStart
    // is the visually-top bubble of a run and isRunEnd the bottom one;
    // run ends keep the full 18dp corner on their outer edge.
    val base = 18.dp
    val tight = 4.dp
    val bubbleShape = if (isMine) {
        RoundedCornerShape(
            topStart = base,
            topEnd = if (item.isRunStart) base else tight,
            bottomEnd = if (item.isRunEnd) base else tight,
            bottomStart = base,
        )
    } else {
        RoundedCornerShape(
            topStart = if (item.isRunStart) base else tight,
            topEnd = base,
            bottomEnd = base,
            bottomStart = if (item.isRunEnd) base else tight,
        )
    }
    // Long-press opens the floating capsule — acked messages only: a
    // pending row has no server id to react to yet. The bubble's window
    // bounds are captured on every placement so the capsule anchors to
    // where the bubble actually is (nothing reads the state during
    // composition, so scroll churn does not recompose); each placement
    // also forwards the bounds via onPositioned so an already-open
    // capsule can re-anchor when the list shifts. clip keeps
    // pressed content inside the bubble's rounded shape; indication is
    // null because the onClick is empty (kept only so combinedClickable
    // preserves the long-press semantics) — a casual tap should not
    // ripple, the capsule's spring-out is the long-press feedback.
    var bubbleBounds by remember { mutableStateOf(Rect.Zero) }
    // Detected links are resolved once here because two places need
    // them: BubbleContent draws and hit-tests them, and the bubble's
    // semantics below turns each into a custom action — the tap
    // detector is a raw pointerInput, which contributes no click action,
    // so without these the links would be sighted-only.
    val emojiFontSize = remember(entity.body) { EmojiOnly.displayFontSize(entity.body) }
    // MARKDOWN FIRST, and the order is load-bearing. Markdown DELETES
    // characters (`**`, backticks, `](url)`), so detecting links over the
    // raw body and drawing the rendered one would leave every link after
    // the first markup token pointing at the wrong glyphs — silently, with
    // nothing failing. Everything below indexes the block's own
    // `rendered.text`.
    //
    // Almost always ONE block: only a table splits a body, and each block
    // is then a whole offset space of its own — its links index its own
    // string, and the hit test below gets one layout result per block. A
    // span from one block resolved against another's layout would point at
    // whatever glyphs happened to sit at those offsets.
    //
    // Emoji-only bodies branch around it: the ladder's whole subject is
    // that the message is nothing but glyphs, so a markup pass could only
    // take something away, and there is nothing in one to detect. Same rule
    // as iOS and macOS.
    val bodyBlocks = remember(entity.body, emojiFontSize) {
        if (emojiFontSize != null) {
            listOf(BodyBlock(MessageMarkdown.Block.Text(MessageMarkdown.plain(entity.body))))
        } else {
            MessageMarkdown.blocks(entity.body).map { block ->
                when (block) {
                    // The markup's own links first, then whatever the
                    // detector finds in the text NOT already covered by
                    // one. Both index this block's rendered text, and both
                    // feed one hit test.
                    //
                    // The overlap rule is not tidiness. `[https://www.paypal.com](https://evil.example)`
                    // renders the label "https://www.paypal.com", which Linkify then
                    // detects as a link to PayPal — two spans over the same glyphs,
                    // one going somewhere else. Whichever the hit test picked, a tap
                    // could open a destination the reader had every reason to think
                    // was the one they could see. Dropping the detector's overlap
                    // leaves exactly one answer: the destination the author wrote.
                    is MessageMarkdown.Block.Text -> BodyBlock(
                        block = block,
                        links = MessageLinks.mergeSpans(
                            block.rendered.links,
                            MessageLinks.linkSpans(block.rendered.text),
                        ),
                    )
                    // A table's cells carry no links by construction
                    // (MessageMarkdown.cell says why), so there is nothing
                    // inside one to hit-test.
                    is MessageMarkdown.Block.Table -> BodyBlock(block)
                }
            }
        }
    }
    val uriHandler = LocalUriHandler.current
    BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
        val bubbleMaxWidth = maxWidth * 0.8f
        val bubbleModifier = Modifier
            .widthIn(max = bubbleMaxWidth)
            .onGloballyPositioned {
                bubbleBounds = it.boundsInWindow()
                onPositioned(item, bubbleBounds)
            }
            .clip(bubbleShape)
            .then(
                if (bodyBlocks.none { it.links.isNotEmpty() }) {
                    Modifier
                } else {
                    Modifier.semantics {
                        customActions = bodyBlocks.flatMap { bodyBlock ->
                            // The RENDERED text of THIS block, not the raw
                            // body and not some concatenation of all of
                            // them: the label is a slice at the span's own
                            // offsets, and slicing anything else here would
                            // announce the wrong words.
                            val text = (bodyBlock.block as? MessageMarkdown.Block.Text)
                                ?.rendered?.text.orEmpty()
                            bodyBlock.links.map { span ->
                                CustomAccessibilityAction(
                                    MessageLinks.accessibilityLabel(text, span),
                                ) {
                                    runCatching { uriHandler.openUri(span.url) }.isSuccess
                                }
                            }
                        }
                    }
                },
            )
            .then(
                if (entity.serverId != null) {
                    Modifier.combinedClickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = {},
                        // Double-tap = the quick heart (Tapback idiom),
                        // through the same toggle path as the capsule,
                        // so a second double-tap removes it. onClick is
                        // a no-op, so the double-tap wait delays nothing.
                        onDoubleClick = {
                            entity.serverId?.let { onToggleReaction(it, DOUBLE_TAP_REACTION) }
                        },
                        onLongClick = { onLongPress(item, bubbleBounds) },
                    )
                } else {
                    Modifier
                },
            )
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 2.dp),
            horizontalAlignment = if (isMine) Alignment.End else Alignment.Start,
        ) {
            if (item.showSenderName) {
                // The sender's face rides the name line at the head of a
                // run rather than in a gutter beside every bubble: the
                // thread's layout — and the run-corner geometry above —
                // stays exactly as it was. Same choice as iOS.
                val senderName = item.senderName ?: stringResource(R.string.s_member_n, entity.senderId)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(start = 4.dp, bottom = 2.dp),
                ) {
                    Avatar(
                        name = senderName,
                        userId = entity.senderId,
                        size = 20,
                        avatarVersion = memberAvatars[entity.senderId] ?: 0L,
                    )
                    Spacer(Modifier.width(5.dp))
                    Text(
                        text = senderName,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            // Emoji-only messages render bare: no balloon, just the
            // glyphs (padding, long-press target and capsule anchoring
            // unchanged). Content color goes onSurface on both sides so
            // the timestamp/status row stays readable on the bare
            // background. Same treatment as iOS.
            val isEmojiOnly = remember(entity.body) {
                EmojiOnly.displayFontSize(entity.body) != null
            }
            Surface(
                shape = bubbleShape,
                color = when {
                    isEmojiOnly -> Color.Transparent
                    isMine -> MaterialTheme.colorScheme.primaryContainer
                    else -> MaterialTheme.colorScheme.surfaceContainerHigh
                },
                contentColor = if (isMine && !isEmojiOnly) {
                    MaterialTheme.colorScheme.onPrimaryContainer
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
                modifier = bubbleModifier,
            ) {
                BubbleContent(
                    entity = entity,
                    item = item,
                    chat = chat,
                    isMine = isMine,
                    isStreaming = isStreaming,
                    emojiFontSize = emojiFontSize,
                    blocks = bodyBlocks,
                    memberNames = memberNames,
                    memberAvatars = memberAvatars,
                    myUserId = myUserId,
                    linkPreviews = linkPreviews,
                    previewsEnabled = previewsEnabled,
                    mapPreviewsEnabled = mapPreviewsEnabled,
                    onRequestPreview = onRequestPreview,
                    streamUrl = streamUrl,
                    onOpenLink = { runCatching { uriHandler.openUri(it) } },
                    onFailedTap = onFailedTap,
                    onToggleReaction = { emoji ->
                        entity.serverId?.let { onToggleReaction(it, emoji) }
                    },
                    onVote = { optionId ->
                        entity.serverId?.let { onVote(it, optionId) }
                    },
                    onCallBack = onCallBack,
                    onDoubleTap = {
                        entity.serverId?.let { onToggleReaction(it, DOUBLE_TAP_REACTION) }
                    },
                    onTextLongPress = { onLongPress(item, bubbleBounds) },
                    onTapQuote = onTapQuote,
                    onOpenAttachment = onOpenAttachment,
                )
            }
        }
    }
}

/**
 * The quoted message inside a reply's balloon: an accent bar, the author,
 * and the server's excerpt.
 *
 * It draws the SNAPSHOT the server sent rather than looking up a live row —
 * the quoted message may be pages back or not cached at all — so it renders
 * identically whatever this device happens to hold. Jumping is the tap
 * handler's job, and degrades to nothing when the target is not loaded.
 */
/**
 * A scroll intent: which message, and which of the two idioms.
 *
 * A quote tap CENTRES its target and tints it; the opening anchor lands
 * it near the TOP with the unread messages readable below, and tints
 * nothing — the divider is the mark, and a highlight would be a second
 * one saying something slightly different.
 */
private data class JumpRequest(
    val serverId: Long,
    val anchor: Boolean,
)

/**
 * The row an anchored open scrolls to: the "N new messages" divider if
 * the builder has produced it, otherwise the message itself.
 *
 * Recomputed at scroll time and never captured — item indices are not
 * message indices (the builder interleaves date pills and this divider),
 * and a message arriving mid-move shifts every one of them.
 */
private fun anchorIndex(items: List<ChatListItem>, serverId: Long): Int {
    val divider = items.indexOfFirst { it is ChatListItem.NewMessagesDivider }
    if (divider >= 0) return divider
    return items.indexOfFirst {
        it is ChatListItem.MessageItem && it.entity.serverId == serverId
    }
}

/**
 * How far below the top edge an anchored open lands the divider — see
 * anchorTopMarginPx at the call site for why it is not zero.
 */
private val ANCHOR_TOP_MARGIN = 72.dp

/**
 * One frame between an anchored open's two scroll passes: the list has
 * to lay out at the new position (and take on the divider row, if it
 * arrived with the same state change) before the second pass can read a
 * true index. Long enough to be a frame on a slow device, short enough
 * that nobody sees two moves.
 */
private const val ANCHOR_SETTLE_MS = 32L

/**
 * How long an anchored open waits for the list to report a viewport
 * before giving up and opening where it is. Generous — this is a
 * backstop against never settling, not a deadline anybody should hit.
 */
private const val ANCHOR_VIEWPORT_TIMEOUT_MS = 1_000L

/** How long a jumped-to bubble stays tinted. Same as iOS. */
private const val HIGHLIGHT_MS = 1_600L

@Composable
private fun QuoteBlock(
    authorName: String,
    excerpt: String,
    isMine: Boolean,
    /** The second level, already resolved to "<name>: <excerpt>", or null. */
    parentLine: String? = null,
    onClick: () -> Unit,
    onDoubleClick: () -> Unit,
    onLongClick: () -> Unit,
) {
    // On my own balloon the accent runs out of contrast against
    // primaryContainer, so the bar and name take the balloon's content
    // colour there and the theme accent on theirs — the same rule the
    // link colour follows.
    val accent = if (isMine) LocalContentColor.current else MaterialTheme.colorScheme.primary
    Row(
        modifier = Modifier
            .clip(RoundedRectangle6)
            .background(LocalContentColor.current.copy(alpha = 0.08f))
            // combinedClickable, not clickable: a child that only handles
            // taps still CONSUMES the press, so long-pressing or
            // double-tapping over the quote silently did nothing instead of
            // opening the capsule or leaving a heart. The bubble's own
            // gestures are handed through.
            .combinedClickable(
                onClick = onClick,
                onDoubleClick = onDoubleClick,
                onLongClick = onLongClick,
            )
            .padding(end = 6.dp)
            .semantics {
                contentDescription = if (parentLine == null) {
                    "Replying to $authorName: $excerpt"
                } else {
                    "Replying to $authorName: $excerpt, which replied to $parentLine"
                }
                role = Role.Button
            },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier
                .padding(vertical = 4.dp)
                .width(3.dp)
                .height(30.dp)
                .background(accent, RoundedCornerShape(1.5.dp)),
        )
        Spacer(Modifier.width(6.dp))
        Column(modifier = Modifier.padding(vertical = 4.dp)) {
            // The second level first, and quieter: it is context for the
            // quote below it, not the thing being answered. One line only —
            // two levels at two lines each would be a wall of grey above
            // every reply in a busy thread.
            if (parentLine != null) {
                Text(
                    text = parentLine,
                    style = MaterialTheme.typography.labelSmall,
                    color = LocalContentColor.current.copy(alpha = 0.5f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = authorName,
                style = MaterialTheme.typography.labelSmall,
                color = accent,
            )
            Text(
                text = excerpt,
                style = MaterialTheme.typography.bodySmall,
                color = LocalContentColor.current.copy(alpha = 0.75f),
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private val RoundedRectangle6 = RoundedCornerShape(6.dp)

/**
 * The wrapping chip row plus the who-reacted DropdownMenu.
 *
 * A tap never takes a reaction away. On a chip I am not part of it
 * joins that reaction (moving mine if I had a different one — the
 * protocol allows one per user); on a chip I AM part of it opens the
 * who-reacted list instead, where my own row is the explicit "tap to
 * remove". Removing used to ride on the same tap that shows who
 * reacted — miss the long-press threshold by a hair and your reaction
 * was gone — and undoing something you never meant to do is a worse
 * failure than one extra tap. Long-press still opens the list from any
 * chip.
 *
 * New chips scale+fade in; the row animates its size as chips come and
 * go.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ReactionChipsRow(
    item: ChatListItem.MessageItem,
    memberNames: Map<Long, String>,
    memberAvatars: Map<Long, Long>,
    myUserId: Long?,
    isMine: Boolean,
    onToggle: (String) -> Unit,
) {
    // Resolved lazily when the list opens — no per-frame decode of the
    // row's reactionsJson for every visible bubble.
    var details by remember { mutableStateOf<List<ReactionDetail>?>(null) }
    // Reactor display name → user id, captured alongside details so each
    // popup row can lead with that reactor's Avatar (hue keyed by id).
    var reactorIds by remember { mutableStateOf<Map<String, Long>>(emptyMap()) }
    // Window-x of the row and of each chip, deliberately NOT snapshot
    // state: written on every placement, read only inside the long-press
    // handler, so scroll churn never invalidates composition. Feeds the
    // menu's x-offset so it opens under the pressed chip (LTR-only math —
    // the app is hardcoded English).
    val rowLeft = remember { floatArrayOf(0f) }
    val chipLefts = remember { mutableMapOf<String, Float>() }
    var menuOffsetX by remember { mutableStateOf(0.dp) }
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    // Resolved out here: openDetails runs in tap handlers, which are not
    // composable contexts. The same strings feed buildReactionDetails so
    // the popup rows and the avatar lookup agree on the labels.
    val youLabel = stringResource(R.string.s_you)
    val memberTemplate = stringResource(R.string.s_member_n)
    // Opening the list: decode the reactions once, resolve the names,
    // and point the menu at the chip that was pressed.
    val openDetails: (String) -> Unit = { emoji ->
        val reactions = ReactionsCodec.decode(item.entity.reactionsJson)
        reactorIds = buildMap {
            for (reaction in reactions) {
                val name = if (reaction.userId == myUserId) {
                    youLabel
                } else {
                    memberNames[reaction.userId] ?: memberTemplate.format(reaction.userId)
                }
                if (name !in this) put(name, reaction.userId)
            }
        }
        menuOffsetX = with(density) {
            ((chipLefts[emoji] ?: rowLeft[0]) - rowLeft[0]).toDp()
        }
        details = buildReactionDetails(
            reactions = reactions,
            names = memberNames,
            myUserId = myUserId ?: -1L,
            youLabel = youLabel,
            memberFallback = { memberTemplate.format(it) },
        )
    }
    Box(modifier = Modifier.onGloballyPositioned { rowLeft[0] = it.boundsInWindow().left }) {
        FlowRow(
            // Wrapped rows hug the balloon's own edge, like iOS's
            // FlowLayout rowAlignment.
            horizontalArrangement = Arrangement.spacedBy(
                4.dp,
                if (isMine) Alignment.End else Alignment.Start,
            ),
            verticalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier
                // Breathing room between the message and its reactions —
                // they are about the message, not part of it. Matches the
                // 6pt + 2pt the iOS balloon ends up with.
                .padding(top = 8.dp)
                .animateContentSize(),
        ) {
            item.reactionChips.forEach { chip ->
                key(chip.emoji) {
                    // A chip new to this message animates in once; chips
                    // that were already there keep their settled state.
                    // "Already appeared" survives lazy disposal via
                    // rememberSaveable, so a bubble scrolling back into
                    // view re-seeds the transition settled instead of
                    // replaying the entrance.
                    var hasAppeared by rememberSaveable { mutableStateOf(false) }
                    val appeared = remember {
                        MutableTransitionState(hasAppeared).apply { targetState = true }
                    }
                    SideEffect { hasAppeared = true }
                    AnimatedVisibility(
                        visibleState = appeared,
                        enter = fadeIn() + scaleIn(initialScale = 0.5f),
                    ) {
                        ReactionChipView(
                            chip = chip,
                            // Additive tap only; removing goes through
                            // the list (see this composable's header).
                            onTap = {
                                if (chip.includesMe) {
                                    openDetails(chip.emoji)
                                } else {
                                    onToggle(chip.emoji)
                                }
                            },
                            onLongPress = {
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                openDetails(chip.emoji)
                            },
                            modifier = Modifier.onGloballyPositioned {
                                chipLefts[chip.emoji] = it.boundsInWindow().left
                            },
                        )
                    }
                }
            }
        }
        DropdownMenu(
            expanded = details != null,
            onDismissRequest = { details = null },
            offset = DpOffset(x = menuOffsetX, y = 0.dp),
            shape = MaterialTheme.shapes.medium,
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            details?.forEach { detail ->
                // My own row is the remove control — the only place a
                // reaction comes off, so it can never happen by accident.
                // Mine-ness comes from the chip, not from matching the
                // "You" label, which is display text.
                val mine = item.reactionChips.firstOrNull { it.emoji == detail.emoji }
                    ?.includesMe == true
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .then(
                            if (mine) {
                                Modifier.clickable {
                                    onToggle(detail.emoji)
                                    details = null
                                }
                            } else {
                                Modifier
                            },
                        )
                        .padding(horizontal = 16.dp, vertical = 6.dp),
                ) {
                    // The row aggregates one emoji's reactors; its avatar is
                    // the first (my "You" entry resolves to my real name so
                    // the initials stay mine).
                    val leadName = detail.names.firstOrNull() ?: "?"
                    val leadId = reactorIds[leadName] ?: 0L
                    Avatar(
                        name = if (leadName == youLabel) {
                            myUserId?.let { memberNames[it] } ?: leadName
                        } else {
                            leadName
                        },
                        userId = leadId,
                        size = 24,
                        avatarVersion = memberAvatars[leadId] ?: 0L,
                    )
                    Spacer(Modifier.width(10.dp))
                    Text(detail.emoji, style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.width(10.dp))
                    Text(
                        text = detail.names.joinToString(", "),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    if (mine) {
                        Spacer(Modifier.width(12.dp))
                        Text(
                            text = stringResource(R.string.s_tap_to_remove),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ReactionChipView(
    chip: ReactionChip,
    onTap: () -> Unit,
    onLongPress: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(12.dp)
    // No fill. A filled pill has to pick a surface colour, and every
    // choice is wrong somewhere: surfaceContainerLowest is the DARKEST
    // tone in a dark scheme, so it read as a black blob on the balloon,
    // and a wash of the balloon's own colour loses contrast on the
    // tinted side. Transparent sidesteps both, and the emoji and count
    // then inherit the balloon's content colour — the colour the
    // message text already uses, so contrast is whatever the bubble
    // already guarantees.
    val onBubble = LocalContentColor.current
    Surface(
        shape = shape,
        color = Color.Transparent,
        contentColor = onBubble,
        // Full strength, never a wash: this hairline is the ONLY thing
        // separating my reaction from someone else's (a count of 1 draws
        // no number), and the tap rule diverges on exactly that. Not
        // `primary` either — on my own balloon that is close to the
        // background it would sit on.
        border = if (chip.includesMe) BorderStroke(1.dp, onBubble) else null,
        modifier = modifier
            .clip(shape)
            .combinedClickable(onClick = onTap, onLongClick = onLongPress),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
        ) {
            // Explicit, and the same number iOS uses: the two were
            // drifting apart (bodyLarge is 16sp, .footnote is 13pt) for
            // the same UI element.
            Text(
                text = chip.emoji,
                style = MaterialTheme.typography.bodyLarge.copy(fontSize = 18.sp),
            )
            // Count changes pop: old value scales+fades out, new one in;
            // the default SizeTransform animates the width when a chip
            // crosses the 1 ↔ 2 "shows a number at all" boundary.
            AnimatedContent(
                targetState = chip.count,
                transitionSpec = {
                    (fadeIn() + scaleIn(initialScale = 0.5f))
                        .togetherWith(fadeOut() + scaleOut(targetScale = 0.5f))
                },
                label = "reaction-count",
            ) { count ->
                Text(
                    text = if (count > 1) " $count" else "",
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        }
    }
}

/**
 * One block of a rendered body with the links that live INSIDE it.
 *
 * The pairing is the point: [links] index `block`'s own rendered string
 * and are meaningless against any other, so the two travel together from
 * the one place that resolves them down to the one layout result they are
 * hit-tested against. A table block carries none — its cells hold no links.
 */
private data class BodyBlock(
    val block: MessageMarkdown.Block,
    val links: List<LinkSpan> = emptyList(),
)

/**
 * The line a call record draws: an outbound / inbound / missed glyph and
 * the wording for its outcome. A tap calls back where that is possible;
 * the bubble's own double-tap and long-press are re-emitted so the heart
 * and the capsule keep working over it, as they do over a poll.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun CallRecordRow(
    line: CallRecordLine,
    isMine: Boolean,
    onCallBack: (() -> Unit)?,
    onDoubleTap: () -> Unit,
    onLongPress: () -> Unit,
) {
    val icon = when (line) {
        is CallRecordLine.Missed, is CallRecordLine.DeclinedByMe -> Icons.Filled.CallMissed
        is CallRecordLine.NoAnswer, is CallRecordLine.DeclinedByThem -> Icons.Filled.CallMade
        is CallRecordLine.Completed, is CallRecordLine.Failed ->
            if (isMine) Icons.Filled.CallMade else Icons.Filled.CallReceived
    }
    val text = when (line) {
        is CallRecordLine.Completed ->
            stringResource(R.string.s_voice_call_with_duration, CallRecordWording.duration(line.durationSecs))
        CallRecordLine.NoAnswer -> stringResource(R.string.s_no_answer)
        CallRecordLine.Missed -> stringResource(R.string.s_missed_voice_call)
        CallRecordLine.DeclinedByThem -> stringResource(R.string.s_voice_call_declined)
        CallRecordLine.DeclinedByMe -> stringResource(R.string.s_declined_voice_call)
        is CallRecordLine.Failed -> line.durationSecs
            ?.let { stringResource(R.string.s_call_failed_with_duration, CallRecordWording.duration(it)) }
            ?: stringResource(R.string.s_call_failed)
    }
    val missed = line is CallRecordLine.Missed
    Row(
        modifier = Modifier
            .combinedClickable(
                onClick = { onCallBack?.invoke() },
                onDoubleClick = onDoubleTap,
                onLongClick = onLongPress,
            )
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = if (missed) MaterialTheme.colorScheme.error else LocalContentColor.current,
            modifier = Modifier.size(20.dp),
        )
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = if (missed) MaterialTheme.colorScheme.error else LocalContentColor.current,
        )
    }
}

/**
 * The poll inside a bubble: one row per option, each with the share of
 * the vote it holds drawn behind it, the faces of the people who chose
 * it, and its count — then a footer saying how much of the family has
 * answered.
 *
 * Colours are derived from the balloon's own content colour rather than
 * from a theme role, the same rule the reaction chips follow: one set of
 * alphas then works on both tones, where a fixed role loses contrast on
 * one of them under dynamic colour.
 *
 * The gestures are the load-bearing part. A child that only handles taps
 * still CONSUMES the press, so an option row that took `clickable` would
 * silently kill the bubble's double-tap heart and its long-press capsule
 * over the whole poll — which is most of the balloon. It takes
 * `combinedClickable` and hands both of them straight back, exactly as
 * QuoteBlock does.
 *
 * iOS counterpart: the shared poll bubble in
 * ios/FamilyConnect/Views/ConversationView.swift.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PollBlock(
    poll: PollView,
    memberAvatars: Map<Long, Long>,
    /** Acked and still open — a closed poll shows its result and refuses taps. */
    canVote: Boolean,
    onVote: (Long) -> Unit,
    onDoubleTap: () -> Unit,
    onLongPress: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(10.dp)
    val onBubble = LocalContentColor.current
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        poll.options.forEach { option ->
            // The bar grows into its new share rather than jumping, so a
            // vote landing from another phone reads as something that
            // happened rather than as a redraw.
            val fraction by animateFloatAsState(
                targetValue = option.fraction,
                animationSpec = tween(250),
                label = "poll-bar",
            )
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(shape)
                    .background(onBubble.copy(alpha = 0.10f))
                    .then(
                        // Full strength, never a wash: this hairline is
                        // the only thing saying which option is MINE, and
                        // the tap rule diverges on exactly that.
                        if (option.isMine) {
                            Modifier.border(BorderStroke(1.dp, onBubble), shape)
                        } else {
                            Modifier
                        },
                    )
                    .combinedClickable(
                        onClick = { if (canVote) onVote(option.id) },
                        onDoubleClick = onDoubleTap,
                        onLongClick = onLongPress,
                    )
                    // A closed poll's row is not a button: it still
                    // takes a long press for the capsule, but a tap on it
                    // does nothing and must not announce that it does.
                    .then(
                        if (canVote) Modifier.semantics { role = Role.Button } else Modifier,
                    ),
            ) {
                if (fraction > 0f) {
                    // matchParentSize, so the fill takes the height the
                    // CONTENT settled on without contributing to it —
                    // fillMaxHeight alone in a wrap-content Box measures
                    // against the incoming max and blows the row open.
                    Box(modifier = Modifier.matchParentSize()) {
                        Box(
                            modifier = Modifier
                                .fillMaxHeight()
                                .fillMaxWidth(fraction)
                                .background(onBubble.copy(alpha = 0.18f)),
                        )
                    }
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                ) {
                    if (option.isMine) {
                        Icon(
                            imageVector = Icons.Outlined.CheckCircle,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                    }
                    Text(
                        text = option.text,
                        style = MaterialTheme.typography.bodyMedium,
                        // fill = true, so the faces and the count are
                        // pushed to the row's far edge instead of
                        // trailing whatever length the text happened to be.
                        modifier = Modifier.weight(1f),
                    )
                    Spacer(Modifier.width(8.dp))
                    // Who voted, by their face — bounded, because an
                    // option every one of nine members chose must not
                    // push the count off the row. Mine leads
                    // (buildPollView), and everybody the row has no space
                    // for is behind the "+N" that ends the strip.
                    if (option.voters.isNotEmpty()) {
                        VoterFaces(
                            voters = option.voters,
                            memberAvatars = memberAvatars,
                            onDoubleTap = onDoubleTap,
                            onLongPress = onLongPress,
                        )
                    }
                    if (option.count > 0) {
                        Text(
                            text = "${option.count}",
                            style = MaterialTheme.typography.labelMedium,
                            color = onBubble.copy(alpha = 0.72f),
                        )
                    }
                }
            }
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(top = 2.dp),
        ) {
            Text(
                // Who has NOT answered is the useful number in a family,
                // and it needs a denominator — so the roster's size, and
                // only the plain count until the roster has answered.
                // Plurals, not format strings: German, Spanish, French,
                // Russian and both Serbian scripts inflect on the count even
                // though English does not. The quantity is the vote count in
                // both, and it is passed twice — once to pick the form, once
                // to fill the %1$d.
                text = if (poll.familySize > 0) {
                    pluralStringResource(
                        R.plurals.s_voted_of_family,
                        poll.votedCount,
                        poll.votedCount,
                        poll.familySize)
                } else {
                    pluralStringResource(
                        R.plurals.s_voted_total, poll.votedCount, poll.votedCount)
                },
                style = MaterialTheme.typography.labelSmall,
                color = onBubble.copy(alpha = 0.72f),
            )
            if (poll.closed) {
                Text(
                    text = " · " + stringResource(R.string.s_poll_closed),
                    style = MaterialTheme.typography.labelSmall,
                    color = onBubble.copy(alpha = 0.72f),
                )
            }
        }
    }
}

/** Faces drawn beside one option before the count takes over. */
private const val MAX_POLL_VOTER_FACES = 4

/**
 * The faces beside one poll option — and the list of everybody behind
 * them.
 *
 * Four faces fit on a row that also has to hold the option's own text.
 * On an option nine members chose, the fifth voter onward used to exist
 * only inside the count, which quietly undoes the decision this feature
 * was built on: in a family, WHO voted is the interesting half. So the
 * strip ends in a "+5" and opens the full list.
 *
 * The list is the same surface the reaction chips already use — a
 * DropdownMenu anchored under what was pressed, one row per person, face
 * first — rather than a second kind of answer to the same question. No
 * header: the menu hangs off the option it belongs to.
 *
 * The gestures are the load-bearing part, exactly as in [PollBlock].
 * This strip sits INSIDE the option row, and a child that handles a tap
 * still CONSUMES the press, so it takes `combinedClickable` and hands
 * the double-tap and the long-press straight back to the balloon — the
 * quick heart and the reaction capsule survive over the faces too. Only
 * the plain tap is claimed, and it deliberately does NOT vote: an
 * affordance that exists to show you something must not also cast a
 * ballot, and the rest of the row is still there for voting.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun VoterFaces(
    voters: List<PollVoter>,
    memberAvatars: Map<Long, Long>,
    onDoubleTap: () -> Unit,
    onLongPress: () -> Unit,
) {
    var showVoters by remember { mutableStateOf(false) }
    val onBubble = LocalContentColor.current
    val hidden = voters.size - MAX_POLL_VOTER_FACES
    // One announcement for the whole strip: a merged node reading five
    // sets of initials tells a screen reader nothing.
    val label = stringResource(R.string.s_who_voted)
    Box {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier
                .combinedClickable(
                    onClick = { showVoters = true },
                    onDoubleClick = onDoubleTap,
                    onLongClick = onLongPress,
                )
                .semantics(mergeDescendants = true) {
                    contentDescription = label
                    role = Role.Button
                },
        ) {
            voters.take(MAX_POLL_VOTER_FACES).forEach { voter ->
                Avatar(
                    name = voter.name,
                    userId = voter.userId,
                    size = 18,
                    avatarVersion = memberAvatars[voter.userId] ?: 0L,
                )
                Spacer(Modifier.width(2.dp))
            }
            if (hidden > 0) {
                // A bare count, like the option's own total beside it —
                // nothing to translate, and it inflects in no language.
                Text(
                    text = "+$hidden",
                    style = MaterialTheme.typography.labelMedium,
                    color = onBubble.copy(alpha = 0.72f),
                )
                Spacer(Modifier.width(4.dp))
            }
        }
        DropdownMenu(
            expanded = showVoters,
            onDismissRequest = { showVoters = false },
            shape = MaterialTheme.shapes.medium,
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            // Everybody, in the order the option holds them — mine first
            // (buildPollView), the same order the faces are drawn in.
            voters.forEach { voter ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                ) {
                    Avatar(
                        name = voter.name,
                        userId = voter.userId,
                        size = 24,
                        avatarVersion = memberAvatars[voter.userId] ?: 0L,
                    )
                    Spacer(Modifier.width(10.dp))
                    Text(text = voter.name, style = MaterialTheme.typography.bodyMedium)
                }
            }
        }
    }
}

/**
 * The poll composer: a question and two to ten options.
 *
 * A sheet rather than a screen because it is one message being written —
 * the composer is still behind it, and the reply it was primed with
 * travels with the poll. Everything it refuses is what the SERVER would
 * refuse (see PollDraft), so Send is dark until the poll is one the
 * family could actually answer, rather than lighting up and failing.
 *
 * The draft lives in the ViewModel, so a rotation with a half-written
 * poll in it loses nothing.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PollComposerSheet(
    draft: PollDraft,
    onQuestionChange: (String) -> Unit,
    onOptionChange: (Int, String) -> Unit,
    onAddOption: () -> Unit,
    onRemoveOption: (Int) -> Unit,
    onSend: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp)
                .padding(bottom = 24.dp)
                // The option list can outgrow the sheet on a small screen
                // with ten of them and the keyboard up.
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = stringResource(R.string.s_new_poll),
                style = MaterialTheme.typography.titleLarge,
            )
            OutlinedTextField(
                value = draft.question,
                onValueChange = onQuestionChange,
                label = { Text(stringResource(R.string.s_question)) },
                singleLine = false,
                modifier = Modifier.fillMaxWidth(),
            )
            draft.options.forEachIndexed { index, option ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    OutlinedTextField(
                        value = option,
                        onValueChange = { onOptionChange(index, it) },
                        placeholder = { Text(stringResource(R.string.s_option)) },
                        singleLine = true,
                        modifier = Modifier.weight(1f),
                    )
                    // Never below two: one option is not a question.
                    if (draft.canRemoveOption) {
                        IconButton(onClick = { onRemoveOption(index) }) {
                            Icon(
                                imageVector = Icons.Filled.Close,
                                contentDescription = stringResource(R.string.s_remove_option),
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
            if (draft.canAddOption) {
                TextButton(onClick = onAddOption) {
                    Icon(Icons.Filled.Add, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.s_add_option))
                }
            }
            Row(
                horizontalArrangement = Arrangement.End,
                modifier = Modifier.fillMaxWidth(),
            ) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.s_cancel))
                }
                Spacer(Modifier.width(8.dp))
                Button(onClick = onSend, enabled = draft.isValid) {
                    Text(stringResource(R.string.s_send))
                }
            }
        }
    }
}

@Composable
private fun BubbleContent(
    entity: MessageEntity,
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    /** The assistant is still writing into this row. */
    isStreaming: Boolean,
    /** Emoji-ladder size for an emoji-only body, else null. Resolved by the caller. */
    emojiFontSize: Float?,
    /**
     * The body with markdown applied, and the links in each block.
     * Resolved by the caller, because the bubble's semantics need the same
     * rendered text the drawing does — every offset below indexes its own
     * block's `rendered.text`, never `entity.body`. A body with no table is
     * exactly one block, which is what keeps this the code path it always
     * was for the messages people actually send.
     */
    blocks: List<BodyBlock>,
    memberNames: Map<Long, String>,
    memberAvatars: Map<Long, Long>,
    myUserId: Long?,
    /** Preview state for every link the app has looked at, keyed by URL. */
    linkPreviews: Map<String, LinkPreviewState>,
    /** False = this device never requests a linked page (Settings). */
    previewsEnabled: Boolean,
    /** Whether a shared location draws a map — the reader's own setting. */
    mapPreviewsEnabled: Boolean,
    onRequestPreview: (String) -> Unit,
    /** Where audio streams from, with the auth header it needs. */
    streamUrl: suspend (Long) -> Pair<String, Map<String, String>>?,
    onOpenLink: (String) -> Unit,
    onFailedTap: (String) -> Unit,
    /** Toggle one emoji for me on this message (no-op until the message is acked). */
    onToggleReaction: (String) -> Unit,
    /** Cast or clear my vote on one poll option (no-op until acked). */
    onVote: (Long) -> Unit,
    /** Tapping a call record calls the other person back; null where that is not possible. */
    onCallBack: (() -> Unit)? = null,
    /** Double-tap over link text — the quick heart, same as the bubble's own gesture. */
    onDoubleTap: () -> Unit,
    /** Long-press over link text — opens the reaction capsule, same as the bubble's own gesture. */
    onTextLongPress: () -> Unit,
    /** Tapping the quote asks to jump to the quoted message. */
    onTapQuote: (Long) -> Unit,
    /** Tapping a photo or video opens it full screen. */
    onOpenAttachment: (AttachmentDto) -> Unit,
) {
    // Everything that is CONTENT shares one left edge, whichever side the
    // balloon is on — the quote, the attachment, the body and the link
    // card — with ONE deliberate exception the product owner reversed:
    // in MY OWN replies the BODY text is right-aligned again (the block
    // sits at End and wrapped lines take TextAlign.End), while the QUOTE
    // above it keeps the left edge, so its accent bar still points at
    // the first character of the excerpt. Everything else stays
    // left-aligned for the original reason: aligning ALL content to the
    // balloon's own side floated a short quote's accent bar away from
    // the text it points at.
    //
    // The chip row and the timestamp still hug the balloon's edge, and they
    // now say so themselves — see their `align` modifiers below. iOS makes
    // the same split with a nested leading VStack.
    Column(
        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.Start,
    ) {
        // The quote sits inside the balloon, above the reply's own text —
        // same placement as iOS.
        val quotedId = entity.replyToMessageId
        val quotedSender = entity.replySenderId
        val quotedExcerpt = entity.replyExcerpt
        if (quotedId != null && quotedSender != null && quotedExcerpt != null) {
            // The same all-three-or-nothing rule one level down. A missing
            // second level is expected: the quoted message was not itself a
            // reply, or its own parent has been swept by retention.
            val parentSender = entity.replyParentSenderId
            val parentExcerpt = entity.replyParentExcerpt
            val parentLine = if (parentSender != null && parentExcerpt != null) {
                val name = when (parentSender) {
                    myUserId -> stringResource(R.string.s_you)
                    else -> memberNames[parentSender] ?: stringResource(R.string.s_someone)
                }
                "$name: $parentExcerpt"
            } else {
                null
            }
            QuoteBlock(
                authorName = when (quotedSender) {
                    myUserId -> stringResource(R.string.s_you)
                    else -> memberNames[quotedSender] ?: stringResource(R.string.s_someone)
                },
                excerpt = quotedExcerpt,
                isMine = isMine,
                parentLine = parentLine,
                onClick = { onTapQuote(quotedId) },
                onDoubleClick = onDoubleTap,
                onLongClick = onTextLongPress,
            )
            Spacer(Modifier.height(4.dp))
        }
        // Emoji-only messages render on the EmojiOnly size ladder (one
        // emoji biggest through four smallest); everything else is body
        // text. Same ladder as iOS. lineHeight goes back to the font's
        // own metrics — bodyMedium's fixed line height would clip a
        // 96sp glyph.
        // Detected links underline; on my bubble they keep the content
        // color (primary can run out of contrast on primaryContainer
        // under dynamic color), on theirs they take primary — mirrors
        // iOS (white vs accent).
        val linkColor = if (isMine) LocalContentColor.current else MaterialTheme.colorScheme.primary
        val mentionColor = if (isMine) LocalContentColor.current else MaterialTheme.colorScheme.primary
        val acked = entity.serverId != null
        // The detector coroutines outlive recomposition and keep the
        // lambda instance they started with (pointerInput only restarts on
        // a KEY change, and these keys never move), so the callbacks are
        // read through rememberUpdatedState — captured directly, the
        // capsule would reopen against the `item` snapshot from this
        // bubble's very first touch and show a stale reaction.
        val currentDoubleTap by rememberUpdatedState(onDoubleTap)
        val currentLongPress by rememberUpdatedState(onTextLongPress)
        // The width of whatever else is in this balloon — a photo, a link
        // card. Text left to itself wraps to the width it WANTS (Compose
        // balances the lines), which under a wide card reads as a narrow
        // paragraph floating over it. Measuring the block and setting it as
        // the text's MINIMUM makes the text wrap against the same edge
        // without widening the balloon, which fillMaxWidth would do: the
        // card wraps its content here, so the balloon is card-width, not
        // the full constraint.
        //
        // Keyed to the message: LazyColumn reuses slots, and a stale width
        // from the previous occupant would stretch an unrelated bubble.
        var blockWidth by remember(entity.clientMsgId) { mutableIntStateOf(0) }
        val measureBlock = Modifier.onSizeChanged { size ->
            if (size.width > blockWidth) blockWidth = size.width
        }

        // The attachments sit above the caption, inside the balloon — one
        // exactly as before, an album as a grid, files and audio as rows
        // (see AttachmentGroup).
        val bubbleAttachments = entity.attachmentList
        if (bubbleAttachments.isNotEmpty()) {
            AttachmentGroup(
                attachments = bubbleAttachments,
                showMapPreviews = mapPreviewsEnabled,
                onOpen = onOpenAttachment,
                modifier = measureBlock,
                streamUrl = streamUrl,
                onLongPress = onTextLongPress,
                onDoubleTap = onDoubleTap,
            )
            if (entity.body.isNotEmpty()) Spacer(Modifier.height(6.dp))
        }
        // The row exists but nothing has arrived yet: a bare cursor says
        // "working" where an empty balloon looks broken. iOS and macOS draw
        // the same thing.
        if (isStreaming && entity.body.isEmpty()) {
            StreamingCursor()
        }
        // A call record draws its own wording from the outcome and the
        // side, never the body — which is the server's English placeholder
        // for clients that predate calls (docs/protocol.md, "Voice calls").
        val callRecord = entity.call
        if (callRecord != null) {
            CallRecordRow(
                line = CallRecordWording.line(callRecord, isMine = isMine),
                isMine = isMine,
                onCallBack = onCallBack,
                onDoubleTap = onDoubleTap,
                onLongPress = onTextLongPress,
            )
        }
        // A photo needs no caption, and an empty Text would still take a
        // line's height inside the balloon.
        if (entity.body.isNotEmpty() && callRecord == null) {
            val minTextWidth = with(LocalDensity.current) { blockWidth.toDp() }
            // The one alignment exception — see the header comment: my
            // own REPLY right-aligns its body under the left-aligned
            // quote. Only plain-text blocks flip; a table keeps its
            // full-width footprint and per-column alignment.
            val bodyAlignsEnd = isMine &&
                quotedId != null && quotedSender != null && quotedExcerpt != null
            blocks.forEachIndexed { index, bodyBlock ->
                // Blocks stack, and the newline that separated them in the
                // source went with the split — this gap stands in for it.
                if (index > 0) Spacer(Modifier.height(6.dp))
                when (val block = bodyBlock.block) {
                    is MessageMarkdown.Block.Text -> TextBlock(
                        rendered = block.rendered,
                        links = bodyBlock.links,
                        // The cursor rides the LAST block, and only when
                        // that block is text — see below for a body that
                        // ends in a table.
                        showCursor = isStreaming && index == blocks.lastIndex,
                        emojiFontSize = emojiFontSize,
                        linkColor = linkColor,
                        mentionColor = mentionColor,
                        minWidth = minTextWidth,
                        acked = acked,
                        textAlign = if (bodyAlignsEnd) TextAlign.End else null,
                        modifier = if (bodyAlignsEnd) Modifier.align(Alignment.End) else Modifier,
                        onDoubleTap = onDoubleTap,
                        onLongPress = onTextLongPress,
                    )
                    // Width-greedy on purpose: the table takes the balloon's
                    // whole width, `measureBlock` reports it, and the text
                    // blocks wrap against that instead of floating narrow
                    // beside a wide grid.
                    is MessageMarkdown.Block.Table -> MarkdownTable(
                        table = block,
                        modifier = measureBlock.fillMaxWidth(),
                    )
                }
            }
            // The assistant is still writing and the last thing rendered is
            // a grid: the cursor becomes a block of its own rather than
            // landing inside a cell.
            if (isStreaming && blocks.lastOrNull()?.block is MessageMarkdown.Block.Table) {
                Spacer(Modifier.height(6.dp))
                StreamingCursor()
            }
        }
        // The poll, under its own question — which IS the body above,
        // so there is nothing to draw twice. A poll with no acked message
        // yet (the optimistic row of one just sent) draws its options and
        // refuses taps: there is no message id to vote against.
        item.poll?.let { poll ->
            if (entity.body.isNotEmpty()) Spacer(Modifier.height(8.dp))
            PollBlock(
                poll = poll,
                memberAvatars = memberAvatars,
                canVote = acked && !poll.closed,
                onVote = onVote,
                onDoubleTap = onDoubleTap,
                onLongPress = onTextLongPress,
                modifier = measureBlock.fillMaxWidth(),
            )
        }
        // The first web link's preview, once it has landed. Asking for
        // it is what starts the fetch — gated on the setting, so a
        // switched-off device never touches the linked site.
        //
        // Over the BLOCKS rather than over the resolved link spans: a
        // table's cells carry no spans by construction, so sourcing the
        // URL from the spans alone left a URL typed into a cell with no
        // card and no other way in — the cell is not tappable either.
        // See MessageLinks.firstDrawnWebLinkUrl.
        val previewUrl = remember(blocks) {
            MessageLinks.firstDrawnWebLinkUrl(blocks.map { it.block })
        }
        if (previewUrl != null && previewsEnabled) {
            LaunchedEffect(previewUrl) { onRequestPreview(previewUrl) }
            val state = linkPreviews[previewUrl]
            if (state is LinkPreviewState.Loaded) {
                LinkPreviewCard(
                    preview = state.preview,
                    image = state.image,
                    onOpen = onOpenLink,
                    modifier = measureBlock,
                    // Only once acked — there is no message id to react to
                    // before that, same gate the text detector uses.
                    onDoubleTap = if (acked) ({ currentDoubleTap() }) else null,
                    onLongPress = { currentLongPress() },
                )
            }
        }

        // Chips live INSIDE the balloon, under the text and above the
        // timestamp: they belong to the message, and outside they made
        // the bubble's footprint ragged. Their own colours are derived
        // from the bubble's content colour, so one rule works on both
        // tones (a primaryContainer chip is invisible on my bubble).
        if (item.reactionChips.isNotEmpty()) {
            // Boxed only to carry the alignment: `ReactionChipsRow` takes
            // no modifier, and the chips have to keep hugging the same edge
            // as the timestamp now the Column itself is Start-aligned.
            Box(modifier = Modifier.align(if (isMine) Alignment.End else Alignment.Start)) {
                ReactionChipsRow(
                    item = item,
                    memberNames = memberNames,
                    memberAvatars = memberAvatars,
                    myUserId = myUserId,
                    isMine = isMine,
                    onToggle = onToggleReaction,
                )
            }
        }
        if (item.showTimestamp || isMine) {
            Spacer(Modifier.size(2.dp))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.End,
                modifier = Modifier.align(Alignment.End),
            ) {
                if (item.showTimestamp) {
                    Text(
                        text = TimeFormat.bubbleTime(entity.createdAt),
                        style = MaterialTheme.typography.labelSmall,
                        // Derived from the bubble's content color, not a
                        // fixed role — onSurfaceVariant mismatches
                        // primaryContainer under dynamic color.
                        color = LocalContentColor.current.copy(alpha = 0.72f),
                    )
                }
                if (entity.editSeq > 0) {
                    // Beside the timestamp, not inside the balloon: it is
                    // metadata about the message, like the time and the
                    // delivery tick, not part of what was said. Same
                    // placement as iOS.
                    if (item.showTimestamp) Spacer(Modifier.width(4.dp))
                    Text(
                        text = stringResource(R.string.s_edited),
                        style = MaterialTheme.typography.labelSmall,
                        color = LocalContentColor.current.copy(alpha = 0.72f),
                    )
                }
                if (isMine) {
                    Spacer(Modifier.width(4.dp))
                    StatusGlyph(entity = entity, chat = chat, onFailedTap = onFailedTap)
                }
            }
        }
    }
}

/** The assistant's "still writing" mark, drawn where the text will land. */
@Composable
private fun StreamingCursor() {
    Text(
        text = "▍",
        style = MaterialTheme.typography.bodyMedium,
        color = LocalContentColor.current.copy(alpha = 0.6f),
    )
}

/**
 * One text block of a body: the string as drawn, and the tap arbitration
 * over it.
 *
 * ONE `TextLayoutResult` and ONE offset space, which is why this is a
 * composable and not a loop body — [links] index [rendered]`.text` and
 * nothing else, so a body split by a table needs a layout result per
 * block. Share one between two and every tap in the second resolves
 * against the first one's glyphs, silently opening the wrong destination.
 *
 * The gesture detector stays a raw `pointerInput` (never
 * `LinkAnnotation.Url`, see [MessageLinks]) and arbitrates all three
 * gestures itself: a linked body must still heart on double-tap and open
 * the reaction capsule on long-press. Blocks with no links skip it
 * entirely and let the balloon's own `combinedClickable` handle them —
 * which is every ordinary message.
 */
@Composable
private fun TextBlock(
    rendered: MessageMarkdown.Rendered,
    /** Links in THIS block's rendered text. */
    links: List<LinkSpan>,
    /** The assistant's cursor rides this block — the last one only. */
    showCursor: Boolean,
    /** Emoji-ladder size for an emoji-only body, else null. */
    emojiFontSize: Float?,
    linkColor: Color,
    mentionColor: Color,
    /** What else in the balloon is already this wide — see `measureBlock`. */
    minWidth: Dp,
    /** Acked: there is a message id to react to. */
    acked: Boolean,
    /** End for the body of my own reply — see BubbleContent's header comment. */
    textAlign: TextAlign? = null,
    modifier: Modifier = Modifier,
    onDoubleTap: () -> Unit,
    onLongPress: () -> Unit,
) {
    val body = remember(rendered, links, linkColor, mentionColor, showCursor) {
        val linked = MessageLinks.styled(
            rendered.annotated,
            links,
            SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline),
        )
        val styled = MessageLinks.withMentions(
            linked,
            SpanStyle(color = mentionColor, fontWeight = FontWeight.Bold),
        )
        // While the assistant is writing, the text ends in a cursor
        // rather than just stopping mid-word — the same signal iOS and
        // macOS give. APPENDED, never inserted: every link offset in
        // `links` indexes this block, and the hand-rolled hit test below
        // matches those offsets against this laid-out string, so anything
        // added ahead of them would silently misdirect taps.
        if (showCursor) buildAnnotatedString { append(styled); append("▍") } else styled
    }
    var textLayout by remember { mutableStateOf<TextLayoutResult?>(null) }
    val uriHandler = LocalUriHandler.current
    // The detector coroutine outlives recomposition and keeps the lambda
    // instance it started with (pointerInput only restarts on a KEY
    // change, and these keys never move), so the callbacks are read
    // through rememberUpdatedState — captured directly, the capsule would
    // reopen against the snapshot from this bubble's very first touch and
    // show a stale reaction.
    val currentDoubleTap by rememberUpdatedState(onDoubleTap)
    val currentLongPress by rememberUpdatedState(onLongPress)
    val textModifier = if (links.isEmpty()) {
        Modifier
    } else {
        Modifier.pointerInput(links, acked) {
            detectTapGestures(
                onTap = { position ->
                    val layout = textLayout ?: return@detectTapGestures
                    // Resolves a tap to a character offset and opens the
                    // span under it (glyph-box check so the empty space
                    // past a short line does not count as its last link).
                    val span = linkSpanAt(layout, links, position)
                        ?: return@detectTapGestures
                    // openUri throws when no app handles the scheme
                    // (tel: on some tablets) — a dead tap beats a crash.
                    runCatching { uriHandler.openUri(span.url) }
                },
                onDoubleTap = if (acked) ({ currentDoubleTap() }) else null,
                onLongPress = if (acked) ({ currentLongPress() }) else null,
            )
        }
    }
    Text(
        text = body,
        onTextLayout = { textLayout = it },
        // linkSpanAt queries the laid-out result (getLineLeft/Right,
        // getBoundingBox), which already reflects the alignment — no
        // offset compensation needed for an End-aligned block.
        textAlign = textAlign,
        modifier = modifier.then(textModifier).widthIn(min = minWidth),
        style = if (emojiFontSize != null) {
            MaterialTheme.typography.bodyMedium.copy(
                fontSize = emojiFontSize.sp,
                lineHeight = TextUnit.Unspecified,
            )
        } else {
            MaterialTheme.typography.bodyMedium
        },
    )
}

/**
 * A pipe table inside a balloon: a Column of weighted Rows, header bold
 * over a hairline rule.
 *
 * It NEVER scrolls horizontally. The columns share the balloon's width by
 * weight and the cells wrap, which costs a few more lines of height and
 * nothing else — where a nested scrollable inside the bounded non-lazy
 * window the Apple clients draw the same conversation in is the exact
 * shape that produced two captured hang reports. Cells hold no links and
 * no gesture of their own, so the balloon's `combinedClickable` still
 * carries the heart and the capsule over the whole grid.
 */
@Composable
private fun MarkdownTable(table: MessageMarkdown.Block.Table, modifier: Modifier = Modifier) {
    Column(modifier = modifier) {
        MarkdownTableRow(cells = table.header, alignments = table.alignments, header = true)
        HorizontalDivider(
            // Derived from the bubble's content color, like the timestamp:
            // outlineVariant mismatches primaryContainer under dynamic color.
            color = LocalContentColor.current.copy(alpha = 0.3f),
            thickness = Dp.Hairline,
        )
        for (row in table.rows) {
            MarkdownTableRow(cells = row, alignments = table.alignments, header = false)
        }
    }
}

@Composable
private fun MarkdownTableRow(
    cells: List<AnnotatedString>,
    alignments: List<TextAlign>,
    header: Boolean,
) {
    Row(modifier = Modifier.fillMaxWidth()) {
        cells.forEachIndexed { index, cell ->
            Text(
                text = cell,
                style = MaterialTheme.typography.bodyMedium,
                // Only the base weight: a cell's own `**bold**` spans still
                // win over it, in the header as well.
                fontWeight = if (header) FontWeight.Bold else null,
                textAlign = alignments.getOrElse(index) { TextAlign.Start },
                modifier = Modifier
                    .weight(1f)
                    .padding(
                        end = if (index == cells.lastIndex) 0.dp else 8.dp,
                        top = 3.dp,
                        bottom = 3.dp,
                    ),
            )
        }
    }
}

/**
 * The preview under a message's first web link: image (when the page
 * offers one), title, description, host.
 *
 * It renders only once the fetch has landed — no skeleton, no reserved
 * space, so a linked bubble changes height once rather than twice.
 * Cut out of the balloon in the app's own low tone for the same reason
 * the reaction chips are: washes of the balloon's own colour lose
 * contrast on one tone or the other.
 *
 * iOS counterpart: ios/FamilyConnect/Views/LinkPreviewCard.swift.
 */
@Composable
private fun LinkPreviewCard(
    preview: LinkPreview,
    image: ImageBitmap?,
    onOpen: (String) -> Unit,
    modifier: Modifier = Modifier,
    /** The bubble's own gestures, forwarded. */
    onDoubleTap: (() -> Unit)? = null,
    onLongPress: () -> Unit = {},
) {
    val shape = RoundedCornerShape(12.dp)
    Surface(
        shape = shape,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
        contentColor = MaterialTheme.colorScheme.onSurface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = modifier
            .padding(top = 6.dp)
            .clip(shape)
            // combinedClickable, not clickable: a plain click handler
            // fired onOpen TWICE on a double tap — the link opened, then
            // opened again — and it consumed the press, so the balloon's
            // heart and reaction capsule never saw it over the card.
            // Compose waits out the double-tap timeout when onDoubleClick
            // is non-null, which is what makes them exclusive.
            .combinedClickable(
                onClickLabel = stringResource(R.string.s_open_link),
                onClick = { onOpen(preview.url) },
                onDoubleClick = onDoubleTap,
                onLongClick = onLongPress,
            ),
    ) {
        Column {
            if (image != null) {
                Image(
                    bitmap = image,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(120.dp),
                )
            }
            Column(modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp)) {
                Text(
                    text = preview.siteName,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = preview.title,
                    style = MaterialTheme.typography.labelLarge,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                preview.description?.let { description ->
                    Text(
                        text = description,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

/**
 * The link span under [position] in ONE laid-out text block, or null when
 * the point misses the text.
 *
 * [layout] and [spans] must come from the SAME block: the offsets are that
 * block's, and resolving them against another block's layout would answer
 * with whatever glyphs happen to sit at those numbers.
 *
 * Neither half of this is what the text APIs hand you directly.
 * getOffsetForPosition answers with the nearest CURSOR BOUNDARY, so a
 * tap on the right half of a glyph rounds UP to the next character —
 * comparing that character's box against the finger rejects the tap and
 * leaves every glyph half dead. And on the y axis the lookup CLAMPS
 * rather than misses, so a point above or below the text resolves to
 * the first or last line at that x; that is reachable here because a
 * pointerInput node also receives taps from the 48dp minimum-touch-
 * target expansion around it, i.e. from the bubble's padding.
 *
 * So: reject anything outside the resolved line's own box, then turn
 * the boundary back into the character actually under the finger.
 */
private fun linkSpanAt(
    layout: TextLayoutResult,
    spans: List<LinkSpan>,
    position: Offset,
): LinkSpan? {
    val length = layout.layoutInput.text.length
    if (length == 0) return null
    val line = layout.getLineForVerticalPosition(position.y)
    if (position.y < layout.getLineTop(line) || position.y > layout.getLineBottom(line)) return null
    if (position.x < layout.getLineLeft(line) || position.x > layout.getLineRight(line)) return null
    val boundary = layout.getOffsetForPosition(position)
    val index = when {
        boundary >= length -> length - 1
        boundary > 0 && position.x < layout.getBoundingBox(boundary).left -> boundary - 1
        else -> boundary
    }
    return spans.firstOrNull { index >= it.start && index < it.end }
}

@Composable
private fun StatusGlyph(
    entity: MessageEntity,
    chat: ChatEntity?,
    onFailedTap: (String) -> Unit,
) {
    // Neutral glyphs follow the bubble's content color (same reasoning
    // as the timestamp); READ keeps primary and FAILED keeps error.
    val metaColor = LocalContentColor.current.copy(alpha = 0.72f)
    // ✓✓ only in direct chats: the family chat has many readers
    // and one peer marker would lie.
    val read = chat?.kind == "direct" &&
        entity.serverId != null &&
        entity.serverId <= (chat.peerLastReadId ?: 0L)
    Crossfade(
        targetState = entity.status to read,
        animationSpec = tween(150),
        label = "statusGlyph",
    ) { (status, isRead) ->
        when (status) {
            MessageStatus.SENDING -> Icon(
                imageVector = Icons.Filled.Schedule,
                contentDescription = stringResource(R.string.s_sending),
                modifier = Modifier.size(14.dp),
                tint = metaColor,
            )
            MessageStatus.SENT -> Icon(
                imageVector = if (isRead) Icons.Filled.DoneAll else Icons.Filled.Check,
                contentDescription = stringResource(
                    if (isRead) R.string.s_read else R.string.s_sent,
                ),
                modifier = Modifier.size(14.dp),
                tint = if (isRead) MaterialTheme.colorScheme.primary else metaColor,
            )
            MessageStatus.FAILED -> Icon(
                imageVector = Icons.Filled.ErrorOutline,
                contentDescription = stringResource(R.string.s_failed_tap_to_retry),
                modifier = Modifier
                    .size(16.dp)
                    .clickable { onFailedTap(entity.clientMsgId) },
                tint = MaterialTheme.colorScheme.error,
            )
        }
    }
}

/** What the composer shows while a photo or video is on its way. */
@Composable
private fun MediaStrip(
    state: ChatViewModel.MediaSendState,
    onDismiss: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        when (state) {
            ChatViewModel.MediaSendState.Preparing,
            ChatViewModel.MediaSendState.Uploading,
            -> {
                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                Text(
                    text = stringResource(
                        if (state == ChatViewModel.MediaSendState.Preparing) {
                            R.string.s_preparing
                        } else {
                            R.string.s_sending_ellipsis
                        },
                    ),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            is ChatViewModel.MediaSendState.Working -> {
                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                Text(
                    text = state.label,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            is ChatViewModel.MediaSendState.Failed -> {
                Icon(
                    imageVector = Icons.Filled.ErrorOutline,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    text = state.reason,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_dismiss)) }
            }
            ChatViewModel.MediaSendState.Idle -> Unit
        }
    }
}

/**
 * Media prepared and waiting for Send, sitting above the field.
 *
 * The thumbnail is the same JPEG the bubble will draw, so what is previewed
 * here is literally what goes out. A file has none — a document is a row,
 * not a tile — and gets its icon instead.
 */
/** While a voice note is being recorded: a counter and the two ways out. */
@Composable
private fun RecordingStrip(
    elapsedMs: Long,
    onStop: () -> Unit,
    onCancel: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            Icons.Filled.Mic,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
        )
        Text(
            text = formatDuration(elapsedMs),
            style = MaterialTheme.typography.bodyMedium,
        )
        Spacer(Modifier.weight(1f))
        TextButton(onClick = onCancel) { Text(stringResource(R.string.s_cancel)) }
        // Stop STAGES rather than sends, so a caption can be added and a
        // recording made by accident can still be discarded.
        TextButton(onClick = onStop) { Text(stringResource(R.string.s_stop)) }
    }
}

/** `0:07` — what a counter counts up in, and what a bubble shows. */
private fun formatDuration(ms: Long): String {
    val whole = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(whole / 60, whole % 60)
}

/**
 * Everything staged, as a horizontally scrolling row of chips — each with
 * its OWN remove, since dropping the third photo of five must not touch
 * the other four. The one-line hint below is shared by the whole set.
 */
@Composable
private fun StagedAttachmentRow(
    staged: List<MediaPrep.Prepared>,
    onDiscard: (Int) -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 12.dp)
                .padding(top = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            staged.forEachIndexed { index, item ->
                StagedAttachmentChip(staged = item, onDiscard = { onDiscard(index) })
            }
        }
        Text(
            text = stringResource(R.string.s_add_a_message_or_send_it_on_its_own),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 2.dp),
        )
    }
}

@Composable
private fun StagedAttachmentChip(
    staged: MediaPrep.Prepared,
    onDiscard: () -> Unit,
) {
    val bitmap = remember(staged) {
        staged.previewJpeg?.let { bytes ->
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
        }
    }
    Row(
        modifier = Modifier.widthIn(max = 220.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(44.dp)
                .clip(RoundedCornerShape(8.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            if (bitmap != null) {
                Image(
                    bitmap = bitmap,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.matchParentSize(),
                )
            } else {
                Icon(
                    Icons.Filled.InsertDriveFile,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (staged.kind == AttachmentDto.KIND_VIDEO) {
                // The same 45%-black circle the bubble thumbnails and grid
                // cells put behind their play glyph (Attachments.kt) — a
                // bare white glyph disappears on a bright first frame.
                Box(
                    modifier = Modifier
                        .size(24.dp)
                        .clip(CircleShape)
                        .background(Color.Black.copy(alpha = 0.45f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Filled.PlayArrow,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                        tint = Color.White,
                    )
                }
            }
        }
        Text(
            text = staged.name?.takeIf { it.isNotBlank() }
                ?: stringResource(
                    if (staged.kind == AttachmentDto.KIND_VIDEO) {
                        R.string.s_video
                    } else {
                        R.string.s_photo
                    },
                ),
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f, fill = false),
        )
        IconButton(onClick = onDiscard) {
            Icon(
                Icons.Filled.Close,
                contentDescription = stringResource(R.string.s_remove_attachment),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun InputBar(
    state: TextFieldState,
    onSend: () -> Unit,
    replyDraft: ReplyToDto?,
    replyAuthorName: String,
    onCancelReply: () -> Unit,
    focusRequester: FocusRequester,
    isEditing: Boolean,
    onCancelEdit: () -> Unit,
    mediaState: ChatViewModel.MediaSendState,
    staged: List<MediaPrep.Prepared>,
    onPickMedia: () -> Unit,
    onPickFile: () -> Unit,
    /** The attach menu's Paste: whatever is on the clipboard, right now. */
    onPasteFromClipboard: () -> Unit,
    /**
     * A clipboard arriving through the field's own paste (or Ctrl+V, or a
     * drop, or a keyboard that inserts pictures). The verdict that comes
     * back is what tells this field how much of it is left to paste.
     */
    onPasteContent: (TransferableContent) -> ChatViewModel.PasteResult,
    /** A paste or a drop was cut short for being longer than a body may be. */
    onPasteTruncated: () -> Unit,
    onTakePhoto: () -> Unit,
    onTakeVideo: () -> Unit,
    onRecordAudio: () -> Unit,
    /**
     * Offer to start a poll. The FAMILY CHAT only — a poll is a family
     * deciding something together, and anywhere else the server answers
     * `invalid_poll` (docs/protocol.md, "Polls").
     */
    showsPoll: Boolean,
    onStartPoll: () -> Unit,
    recordingMs: Long?,
    onStopRecording: () -> Unit,
    onCancelRecording: () -> Unit,
    onDiscardStaged: (Int) -> Unit,
    onDismissMediaError: () -> Unit,
    /** Share where this device is, once. */
    onShareLocation: () -> Unit,
    /**
     * Offer the `@ai` mention. Only in the family chat, and only on a
     * server that has an assistant — an absent `assistant` on
     * `GET /families/mine` is the whole capability check
     * (docs/protocol.md, "Mentioning the assistant in the family chat").
     */
    showsAssistantMention: Boolean,
) {
    Surface(tonalElevation = 3.dp) {
        Column {
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            if (replyDraft != null) {
                ReplyBanner(
                    authorName = replyAuthorName,
                    excerpt = replyDraft.excerpt,
                    onCancel = onCancelReply,
                )
            }
            if (isEditing) {
                EditBanner(onCancel = onCancelEdit)
            }
            if (mediaState != ChatViewModel.MediaSendState.Idle) {
                MediaStrip(state = mediaState, onDismiss = onDismissMediaError)
            }
            if (recordingMs != null) {
                RecordingStrip(
                    elapsedMs = recordingMs,
                    onStop = onStopRecording,
                    onCancel = onCancelRecording,
                )
            }
            if (staged.isNotEmpty()) {
                StagedAttachmentRow(staged = staged, onDiscard = onDiscardStaged)
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                // The state-based field, on purpose: it edits the
                // TextFieldState buffer synchronously (the IME talks to
                // that same buffer), so the ViewModel's clear-on-send
                // cannot race a late IME event resurrecting the sent
                // text — the failure mode of value/onValueChange over
                // an async flow.
                // Editing borrows the composer to rewrite an existing
                // message, which has no second attachment to add.
                // One "attach" intent with two sources — the composer is
                // too narrow for two buttons beside the field.
                var attachMenuOpen by remember { mutableStateOf(false) }
                Box {
                    IconButton(
                        onClick = { attachMenuOpen = true },
                        // isBusy, not "is Idle": a FAILED notice is a
                        // sentence waiting to be dismissed, and it used to
                        // grey this button out — so an error from one paste
                        // blocked the next one until something cleared it.
                        // Greyed at the cap too: a message carries at most
                        // ten attachments, and offering an add that can
                        // only be refused is worse than a disabled button.
                        enabled = !isEditing && !mediaState.isBusy &&
                            staged.size < AttachmentDto.MAX_PER_MESSAGE,
                        modifier = Modifier.size(44.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Filled.AttachFile,
                            contentDescription = stringResource(R.string.s_attach_a_photo_video_or_file),
                        )
                    }
                    DropdownMenu(
                        expanded = attachMenuOpen,
                        onDismissRequest = { attachMenuOpen = false },
                    ) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_photo_or_video)) },
                            leadingIcon = { Icon(Icons.Filled.Image, contentDescription = null) },
                            onClick = {
                                attachMenuOpen = false
                                onPickMedia()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_file)) },
                            leadingIcon = {
                                Icon(Icons.Filled.InsertDriveFile, contentDescription = null)
                            },
                            onClick = {
                                attachMenuOpen = false
                                onPickFile()
                            },
                        )
                        // Inside the menu on purpose: it inherits the
                        // button's guard (no attaching mid-edit or mid-
                        // upload) for free, and it is the door that works
                        // when the text field has no focus at all.
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_paste)) },
                            leadingIcon = {
                                Icon(Icons.Filled.ContentPaste, contentDescription = null)
                            },
                            onClick = {
                                attachMenuOpen = false
                                onPasteFromClipboard()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_take_photo)) },
                            leadingIcon = {
                                Icon(Icons.Filled.PhotoCamera, contentDescription = null)
                            },
                            onClick = {
                                attachMenuOpen = false
                                onTakePhoto()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_record_audio)) },
                            leadingIcon = { Icon(Icons.Filled.Mic, contentDescription = null) },
                            onClick = {
                                attachMenuOpen = false
                                onRecordAudio()
                            },
                        )
                        if (showsPoll) {
                            // Inside the attach menu rather than beside
                            // the field: a poll is one more thing a
                            // message can carry, and it inherits that
                            // button's guard (nothing attaches mid-edit
                            // or mid-upload) for free.
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.s_poll)) },
                                leadingIcon = {
                                    Icon(Icons.Filled.Poll, contentDescription = null)
                                },
                                onClick = {
                                    attachMenuOpen = false
                                    onStartPoll()
                                },
                            )
                        }
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_share_your_location)) },
                            leadingIcon = { Icon(Icons.Filled.Place, contentDescription = null) },
                            onClick = {
                                attachMenuOpen = false
                                onShareLocation()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.s_record_video)) },
                            leadingIcon = {
                                Icon(Icons.Filled.Videocam, contentDescription = null)
                            },
                            onClick = {
                                attachMenuOpen = false
                                onTakeVideo()
                            },
                        )
                    }
                }
                if (showsAssistantMention) {
                    IconButton(
                        onClick = {
                            // Appended, never inserted at the caret: moving
                            // somebody's cursor is worse than adding to the
                            // end of what they were writing, and the phone
                            // and the Mac do the same.
                            val current = state.text.toString()
                            if (!AssistantMention.mentions(current)) {
                                val prefix = when {
                                    current.isEmpty() -> ""
                                    current.endsWith(" ") -> ""
                                    else -> " "
                                }
                                state.edit {
                                    append(prefix + AssistantMention.TOKEN + " ")
                                }
                            }
                            focusRequester.requestFocus()
                        },
                        enabled = !isEditing,
                        modifier = Modifier.size(44.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Filled.AutoAwesome,
                            contentDescription = stringResource(R.string.s_ask_the_assistant),
                        )
                    }
                }
                // The field's OWN paste — the long-press menu, Ctrl+V from
                // a hardware keyboard, a keyboard that inserts pictures,
                // and a drop onto the composer, which all arrive here.
                //
                // No policy lives in this door any more. It used to decide
                // for itself whether attaching was possible (a stricter
                // test than the ViewModel's, so a paste was refused merely
                // because an unrelated error notice was showing) and to
                // pick the item out of the clip itself. Both are the RULE's
                // business now: this only turns the verdict into what
                // Compose wants back — what is LEFT for the field to paste.
                //
                // Through an updated state, so the listener survives a
                // recomposition that only produced a new lambda instance.
                val pasteHandler by rememberUpdatedState(onPasteContent)
                val pasteReceiver = remember {
                    ReceiveContentListener { transferable ->
                        when (pasteHandler(transferable)) {
                            // Staged, or refused because the composer is
                            // busy: either way nothing of this clip belongs
                            // in the field. Consumed whole rather than left
                            // behind — an unconsumed media Uri is dropped
                            // silently by the field's own paste, which is a
                            // gesture that visibly does nothing at all.
                            ChatViewModel.PasteResult.STAGING,
                            ChatViewModel.PasteResult.BUSY,
                            -> null
                            // Words, and anything the rule could not place:
                            // handed back untouched so the field inserts
                            // them where the CARET is. The length cap is
                            // the transformation below, since that
                            // insertion happens inside the field.
                            else -> transferable
                        }
                    }
                }
                // Told once per cut-short paste; typing into a full field
                // stops in silence. See BodyLengthLimit.
                val truncated by rememberUpdatedState(onPasteTruncated)
                val bodyLimit = remember { BodyLengthLimit { truncated() } }
                TextField(
                    state = state,
                    inputTransformation = bodyLimit,
                    // heightIn beats the field's 56.dp defaultMinSize, which
                    // only applies when the incoming min constraint is zero.
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = 44.dp)
                        .focusRequester(focusRequester)
                        .contentReceiver(pasteReceiver),
                    // M3's default content padding for an unlabelled field is
                    // 16.dp top and bottom, which is taller than the 44.dp
                    // buttons' own centring — so both icons sat visibly below
                    // the last line of text. 10.dp puts the text's optical
                    // centre level with them at one line and at five alike.
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
                    placeholder = { Text(stringResource(R.string.s_message)) },
                    lineLimits = TextFieldLineLimits.MultiLine(
                        minHeightInLines = 1,
                        maxHeightInLines = 5,
                    ),
                    shape = RoundedCornerShape(24.dp),
                    colors = TextFieldDefaults.colors(
                        focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                )
                // An attachment can travel with no words at all, so Send is
                // live as soon as there is either.
                val canSend = state.text.isNotBlank() || staged.isNotEmpty()
                // The disabled slots get the same animated colors as the
                // enabled ones — otherwise the tween would be invisible
                // because the button snaps to its disabled palette.
                val sendContainer by animateColorAsState(
                    targetValue = if (canSend) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.surfaceContainerHighest
                    },
                    animationSpec = tween(150),
                    label = "sendContainer",
                )
                val sendContent by animateColorAsState(
                    targetValue = if (canSend) {
                        MaterialTheme.colorScheme.onPrimary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    animationSpec = tween(150),
                    label = "sendContent",
                )
                // Subtle pop the moment the draft first becomes non-blank;
                // scale is draw-only, so neighbors never shift.
                val sendScale by animateFloatAsState(
                    targetValue = if (canSend) 1f else 0.9f,
                    animationSpec = tween(150),
                    label = "sendScale",
                )
                FilledIconButton(
                    onClick = onSend,
                    enabled = canSend,
                    modifier = Modifier
                        .size(44.dp)
                        .scale(sendScale),
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = sendContainer,
                        contentColor = sendContent,
                        disabledContainerColor = sendContainer,
                        disabledContentColor = sendContent,
                    ),
                ) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.Send,
                        contentDescription = stringResource(R.string.s_send),
                    )
                }
            }
        }
    }
}

/**
 * Hand a downloaded attachment to whatever app can read it.
 *
 * The Uri comes from FileProvider: a `file://` one has thrown
 * FileUriExposedException since Android 7, and the grant here is scoped to
 * this single Intent rather than to the directory. Returns false when
 * nothing on the device can open the type — the caller says so rather than
 * letting the tap do nothing.
 */
private fun openWithSystem(context: Context, file: File, mime: String): Boolean {
    val uri = runCatching {
        FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
    }.getOrNull() ?: return false

    val intent = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, mime.ifEmpty { "*/*" })
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    return runCatching { context.startActivity(intent); true }.getOrDefault(false)
}

/**
 * Hand an attachment to the system chooser.
 *
 * ACTION_SEND with a FileProvider Uri: `file://` has thrown since Android
 * 7, and the read grant rides on this one Intent. The caption goes along
 * as EXTRA_TEXT so a photo shared with its words keeps them — apps that
 * only want the stream ignore it.
 *
 * Returns false when nothing on the device can take it, so the caller can
 * say so rather than leaving the tap looking broken.
 */
private fun shareWithSystem(
    context: Context,
    file: File,
    mime: String,
    caption: String,
): Boolean {
    val uri = runCatching {
        FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
    }.getOrNull() ?: return false

    val send = Intent(Intent.ACTION_SEND).apply {
        type = mime.ifEmpty { "*/*" }
        putExtra(Intent.EXTRA_STREAM, uri)
        if (caption.isNotEmpty()) putExtra(Intent.EXTRA_TEXT, caption)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    return runCatching {
        context.startActivity(Intent.createChooser(send, null))
        true
    }.getOrDefault(false)
}
