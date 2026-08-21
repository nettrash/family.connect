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
 * Status glyphs on my bubbles: clock (sending), ✓ (sent), ✓✓ (read —
 * direct chats only, serverId ≤ peerLastReadId), red error → retry/
 * delete dialog. Date pills between days; typing shows on a permanently
 * reserved app-bar subtitle line so the bar never changes height.
 * Scrolling within 10 items of the old end triggers loadOlder (spinner
 * in the list's oldest-end item); a small FAB overlaid above the input
 * bar jumps back to the newest message.
 * Reactions: chips under the bubble (tap toggles, long-press shows who
 * reacted); long-press on an acked bubble opens a floating capsule
 * anchored above it (below near the top) with the quick set + my
 * off-list reaction + a "+" into the full categorized emoji picker
 * sheet (EMOJI_CATALOG).
 *
 * iOS counterpart: ios/FamilyConnect/Views/ConversationView.swift
 */

package me.nettrash.familyconnect.ui.chat

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
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.outlined.Forum
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.ReactionsCodec
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(
    onBack: () -> Unit,
    viewModel: ChatViewModel = hiltViewModel(),
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val chat by viewModel.chat.collectAsStateWithLifecycle()
    val input by viewModel.input.collectAsStateWithLifecycle()
    val typingUser by viewModel.typingUser.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    val isOnline by viewModel.isOnline.collectAsStateWithLifecycle()
    val socketState by viewModel.socketState.collectAsStateWithLifecycle()
    val loadingOlder by viewModel.loadingOlder.collectAsStateWithLifecycle()
    val initialLoadSettled by viewModel.initialLoadSettled.collectAsStateWithLifecycle()

    var failedActionTarget by remember { mutableStateOf<String?>(null) }

    // The message the floating capsule is open for, and the one the "+"
    // full-picker sheet is open for. Both are transient snapshots.
    var pickerTarget by remember { mutableStateOf<ReactionPickerTarget?>(null) }
    var fullPickerTarget by remember { mutableStateOf<ChatListItem.MessageItem?>(null) }
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()

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

    // Pagination: nearing the visually-top (oldest) end triggers one
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
    LaunchedEffect(nearOldEnd) {
        if (nearOldEnd) viewModel.loadOlder()
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
                                    text = "${lastTypingUser.value} is typing…",
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
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
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
                                    title = "No messages yet",
                                    // kind is "family" | "direct" (ChatEntity)
                                    // — a 1:1 chat is not "your family chat".
                                    subtitle = if (chat?.kind == "direct") {
                                        "Say hi — this is where your conversation starts"
                                    } else {
                                        "Say hi — this is where your family chat starts"
                                    },
                                )
                            }
                        }
                    }
                    items(items, key = { it.key }) { item ->
                        Box(
                            modifier = Modifier.animateItem(
                                fadeInSpec = tween(200),
                                placementSpec = spring(stiffness = Spring.StiffnessMediumLow),
                            ),
                        ) {
                            when (item) {
                                is ChatListItem.DateSeparator -> DateSeparatorPill(item.label)
                                is ChatListItem.MessageItem -> MessageBubble(
                                    item = item,
                                    chat = chat,
                                    isMine = item.entity.senderId == myUserId,
                                    myUserId = myUserId,
                                    memberNames = memberNames,
                                    onFailedTap = { failedActionTarget = it },
                                    onToggleReaction = applyToggle,
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
                val showScrollToBottom by remember {
                    derivedStateOf { listState.firstVisibleItemIndex > 5 }
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
                            contentDescription = "Scroll to newest",
                        )
                    }
                }
            }

            InputBar(
                value = input,
                onValueChange = viewModel::onInputChange,
                onSend = viewModel::send,
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
            onPick = { emoji ->
                target.item.entity.serverId?.let { applyToggle(it, emoji) }
            },
            onMore = {
                pickerTarget = null
                fullPickerTarget = target.item
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
                text = "React",
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

    failedActionTarget?.let { clientMsgId ->
        AlertDialog(
            onDismissRequest = { failedActionTarget = null },
            title = { Text("Message not sent") },
            text = { Text("Try sending it again, or delete the draft.") },
            confirmButton = {
                TextButton(onClick = {
                    viewModel.retry(clientMsgId)
                    failedActionTarget = null
                }) {
                    Text("Retry")
                }
            },
            dismissButton = {
                DestructiveTextButton(
                    label = "Delete",
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
    onPick: (String) -> Unit,
    onMore: () -> Unit,
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
        }
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
                    contentDescription = "More reactions",
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

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageBubble(
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    myUserId: Long?,
    memberNames: Map<Long, String>,
    onFailedTap: (String) -> Unit,
    onToggleReaction: (Long, String) -> Unit,
    onLongPress: (ChatListItem.MessageItem, Rect) -> Unit,
    onPositioned: (ChatListItem.MessageItem, Rect) -> Unit,
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
                if (entity.serverId != null) {
                    Modifier.combinedClickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = {},
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
                Text(
                    text = item.senderName ?: "Member ${entity.senderId}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 12.dp, bottom = 2.dp),
                )
            }
            Surface(
                shape = bubbleShape,
                color = if (isMine) {
                    MaterialTheme.colorScheme.primaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceContainerHigh
                },
                contentColor = if (isMine) {
                    MaterialTheme.colorScheme.onPrimaryContainer
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
                modifier = bubbleModifier,
            ) {
                BubbleContent(entity = entity, item = item, chat = chat, isMine = isMine, onFailedTap = onFailedTap)
            }
            // In the bubble's Column (not BubbleContent) so the chips inherit
            // the side alignment of the bubble they belong to.
            if (item.reactionChips.isNotEmpty()) {
                ReactionChipsRow(
                    item = item,
                    memberNames = memberNames,
                    myUserId = myUserId,
                    maxWidth = bubbleMaxWidth,
                    onTap = { emoji -> entity.serverId?.let { onToggleReaction(it, emoji) } },
                )
            }
        }
    }
}

/**
 * The wrapping chip row plus the who-reacted DropdownMenu a chip
 * long-press opens (tap still toggles). New chips scale+fade in; the
 * row animates its size as chips come and go.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ReactionChipsRow(
    item: ChatListItem.MessageItem,
    memberNames: Map<Long, String>,
    myUserId: Long?,
    maxWidth: Dp,
    onTap: (String) -> Unit,
) {
    // Resolved lazily on long-press — no per-frame decode of the row's
    // reactionsJson for every visible bubble.
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
    Box(modifier = Modifier.onGloballyPositioned { rowLeft[0] = it.boundsInWindow().left }) {
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier
                .widthIn(max = maxWidth)
                .padding(top = 4.dp, bottom = 4.dp)
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
                            onTap = { onTap(chip.emoji) },
                            onLongPress = {
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                val reactions = ReactionsCodec.decode(item.entity.reactionsJson)
                                reactorIds = buildMap {
                                    for (reaction in reactions) {
                                        val name = if (reaction.userId == myUserId) {
                                            "You"
                                        } else {
                                            memberNames[reaction.userId]
                                                ?: "Member ${reaction.userId}"
                                        }
                                        if (name !in this) put(name, reaction.userId)
                                    }
                                }
                                menuOffsetX = with(density) {
                                    ((chipLefts[chip.emoji] ?: rowLeft[0]) - rowLeft[0]).toDp()
                                }
                                details = buildReactionDetails(
                                    reactions = reactions,
                                    names = memberNames,
                                    myUserId = myUserId ?: -1L,
                                )
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
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                ) {
                    // The row aggregates one emoji's reactors; its avatar is
                    // the first (my "You" entry resolves to my real name so
                    // the initials stay mine).
                    val leadName = detail.names.firstOrNull() ?: "?"
                    Avatar(
                        name = if (leadName == "You") {
                            myUserId?.let { memberNames[it] } ?: leadName
                        } else {
                            leadName
                        },
                        userId = reactorIds[leadName] ?: 0L,
                        size = 24,
                    )
                    Spacer(Modifier.width(10.dp))
                    Text(detail.emoji, style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.width(10.dp))
                    Text(
                        text = detail.names.joinToString(", "),
                        style = MaterialTheme.typography.bodyMedium,
                    )
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
    Surface(
        shape = shape,
        color = if (chip.includesMe) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        },
        border = if (chip.includesMe) {
            BorderStroke(1.dp, MaterialTheme.colorScheme.primary)
        } else {
            null
        },
        modifier = modifier
            .clip(shape)
            .combinedClickable(onClick = onTap, onLongClick = onLongPress),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
        ) {
            Text(chip.emoji, style = MaterialTheme.typography.bodyLarge)
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

@Composable
private fun BubbleContent(
    entity: MessageEntity,
    item: ChatListItem.MessageItem,
    chat: ChatEntity?,
    isMine: Boolean,
    onFailedTap: (String) -> Unit,
) {
    Column(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
        Text(
            text = entity.body,
            style = MaterialTheme.typography.bodyMedium,
        )
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
                if (isMine) {
                    Spacer(Modifier.width(4.dp))
                    StatusGlyph(entity = entity, chat = chat, onFailedTap = onFailedTap)
                }
            }
        }
    }
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
                contentDescription = "Sending",
                modifier = Modifier.size(14.dp),
                tint = metaColor,
            )
            MessageStatus.SENT -> Icon(
                imageVector = if (isRead) Icons.Filled.DoneAll else Icons.Filled.Check,
                contentDescription = if (isRead) "Read" else "Sent",
                modifier = Modifier.size(14.dp),
                tint = if (isRead) MaterialTheme.colorScheme.primary else metaColor,
            )
            MessageStatus.FAILED -> Icon(
                imageVector = Icons.Filled.ErrorOutline,
                contentDescription = "Failed — tap to retry",
                modifier = Modifier
                    .size(16.dp)
                    .clickable { onFailedTap(entity.clientMsgId) },
                tint = MaterialTheme.colorScheme.error,
            )
        }
    }
}

@Composable
private fun InputBar(
    value: String,
    onValueChange: (String) -> Unit,
    onSend: () -> Unit,
) {
    Surface(tonalElevation = 3.dp) {
        Column {
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                TextField(
                    value = value,
                    onValueChange = onValueChange,
                    // heightIn beats the field's 56.dp defaultMinSize, which
                    // only applies when the incoming min constraint is zero.
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = 44.dp),
                    placeholder = { Text("Message") },
                    minLines = 1,
                    maxLines = 5,
                    shape = RoundedCornerShape(24.dp),
                    colors = TextFieldDefaults.colors(
                        focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                )
                val canSend = value.isNotBlank()
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
                        contentDescription = "Send",
                    )
                }
            }
        }
    }
}
