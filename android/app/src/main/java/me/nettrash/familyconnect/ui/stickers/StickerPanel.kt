/*
 * StickerPanel.kt
 * Family Connect (Android)
 *
 * The three places a person meets the family's stickers (docs/protocol.md,
 * "Sticker pack"), the way a messenger's stickers work:
 *
 *  - THE PANEL, opened from the composer: the pack as a grid, and ONE TAP
 *    SENDS — no caption, no confirmation, the sticker is the message.
 *  - THE LARGER VIEW a tap on a sticker in a chat opens, with "Add to family
 *    stickers" when the pack does not hold it.
 *  - THE PACK'S MANAGEMENT, on the Family screen: anybody adds, and whoever
 *    added an item — or the family owner — removes it.
 *
 * The word "sticker" on this screen is the chat picture. A board note is a
 * "note" to the people using the app and always has been; the two are
 * unrelated.
 *
 * iOS counterpart: the sticker panel and the Family screen's pack section.
 */

package me.nettrash.familyconnect.ui.stickers

import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.outlined.EmojiEmotions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.PackItemEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.PackPicture
import me.nettrash.familyconnect.ui.components.DestructiveTextButton
import me.nettrash.familyconnect.ui.components.EmptyState
import me.nettrash.familyconnect.ui.components.LocalAttachments

/**
 * Say what the sticker model has to say — an add that worked, a pack that
 * is full, a picture that is too big. A toast, like the share flow's: each
 * is one sentence about something that just happened.
 *
 * Composed ONCE per screen. The notices are a SharedFlow, so two collectors
 * on one screen would say everything twice.
 */
@Composable
fun StickerNotices(viewModel: StickerViewModel) {
    val context = LocalContext.current
    LaunchedEffect(viewModel) {
        viewModel.notices.collect { message ->
            Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
        }
    }
}

// -- The panel ----------------------------------------------------------------

/**
 * The family's pack as a grid, in a sheet over the composer. ONE TAP SENDS:
 * [onPick] is the whole of the interaction, and the sheet closes on it so
 * the sticker is seen landing in the chat.
 *
 * Recently used first — this device's own list, never on the wire — and
 * then the pack in the order it was added, which does not shuffle.
 *
 * Every item is shown to every member. A blocked member's pack items are
 * NOT hidden: an item is a picture the family keeps, not something a person
 * said, and a panel one sticker short for one member would be a quantity
 * that moved when they blocked somebody.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StickerPanelSheet(
    viewModel: StickerViewModel,
    onPick: (PackItemEntity) -> Unit,
    onDismiss: () -> Unit,
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val recents by viewModel.recents.collectAsStateWithLifecycle()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        if (items.isEmpty()) {
            EmptyState(
                icon = Icons.Outlined.EmojiEmotions,
                title = stringResource(R.string.s_no_stickers_yet),
                subtitle = stringResource(R.string.s_no_stickers_hint),
                modifier = Modifier.fillMaxWidth(),
            )
        } else {
            StickerGrid(
                items = items,
                recents = recents,
                load = viewModel::packFile,
                onPick = onPick,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 360.dp),
            )
        }
    }
}

/**
 * The grid itself, with no sheet and no ViewModel — so a test can hold it.
 */
@Composable
internal fun StickerGrid(
    items: List<PackItemEntity>,
    recents: List<PackItemEntity>,
    load: suspend (PackItemEntity) -> java.io.File?,
    onPick: (PackItemEntity) -> Unit,
    modifier: Modifier = Modifier,
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(PANEL_CELL),
        modifier = modifier,
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (recents.isNotEmpty()) {
            item(key = "header-recent", span = { GridItemSpan(maxLineSpan) }) {
                PanelHeader(stringResource(R.string.s_recently_used))
            }
            // Keyed apart from the pack below: a recent sticker is in both
            // lists, and a lazy grid refuses two items under one key.
            items(recents, key = { "recent-${it.id}" }) { item ->
                StickerCell(item = item, load = load, onClick = { onPick(item) })
            }
            item(key = "header-pack", span = { GridItemSpan(maxLineSpan) }) {
                PanelHeader(stringResource(R.string.s_family_stickers))
            }
        }
        items(items, key = { "pack-${it.id}" }) { item ->
            StickerCell(item = item, load = load, onClick = { onPick(item) })
        }
    }
}

@Composable
private fun PanelHeader(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 4.dp, vertical = 4.dp),
    )
}

@Composable
private fun StickerCell(
    item: PackItemEntity,
    load: suspend (PackItemEntity) -> java.io.File?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val picture = item.attachment
    // The label is what a screen reader says: the words whoever added it
    // gave, and otherwise just what it is.
    val description = item.label ?: stringResource(R.string.s_sticker)
    val attachments = LocalAttachments.current
    Box(
        modifier = modifier
            .aspectRatio(1f)
            .clip(RoundedCornerShape(12.dp))
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = description }
            .testTag("sticker-${item.id}")
            .padding(6.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (picture != null) {
            StickerImage(
                key = picture.id,
                load = { load(item) },
                // Said once, by the cell.
                contentDescription = null,
                modifier = Modifier.fillMaxSize(),
                retryKey = attachments?.retryToken,
                decodeBox = PANEL_CELL * CELL_GROWTH,
            )
        }
    }
}

// -- The larger view ----------------------------------------------------------

/**
 * A sticker somebody sent, shown larger — and "Add to family stickers" when
 * the family's pack does not hold it.
 *
 * Whether it does is decided here, from bytes this device already has
 * (StickerViewModel.offersAdd); the message names no pack item and needs
 * none. Re-asked whenever the pack changes, so the button goes the moment
 * the add lands — from this device or anybody else's.
 */
@Composable
fun StickerPreviewDialog(
    attachment: AttachmentDto,
    viewModel: StickerViewModel,
    onDismiss: () -> Unit,
) {
    val attachments = LocalAttachments.current
    val items by viewModel.items.collectAsStateWithLifecycle()
    val adding by viewModel.adding.collectAsStateWithLifecycle()
    val offersAdd by produceState(initialValue = false, attachment.id, items) {
        value = viewModel.offersAdd(attachment)
    }
    Dialog(onDismissRequest = onDismiss) {
        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                StickerImage(
                    key = attachment.id,
                    load = { attachments?.originalFile(attachment.id) },
                    contentDescription = stringResource(R.string.s_sticker),
                    modifier = Modifier.size(StickerDrawing.ENLARGED_BOX),
                    retryKey = attachments?.retryToken,
                )
                Spacer(Modifier.height(12.dp))
                if (offersAdd) {
                    Button(
                        onClick = { viewModel.addFromMessage(attachment) },
                        enabled = !adding,
                        modifier = Modifier.testTag("add-to-family-stickers"),
                    ) {
                        Text(stringResource(R.string.s_add_to_family_stickers))
                    }
                }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_close)) }
            }
        }
    }
}

// -- The Family screen --------------------------------------------------------

/**
 * The pack's row on the Family screen, and the manager it opens.
 *
 * Draws NOTHING on a server that predates packs: `GET /families/mine`
 * without `max_pack_items` is how a client knows, and a section that led to
 * a 404 would be worse than no section.
 *
 * For every member, not only the owner — anybody in the family may add.
 */
@Composable
fun FamilyStickersSection(
    modifier: Modifier = Modifier,
    viewModel: StickerViewModel = hiltViewModel(),
) {
    val limits by viewModel.limits.collectAsStateWithLifecycle()
    val ceiling = limits ?: return
    val items by viewModel.items.collectAsStateWithLifecycle()
    var managing by rememberSaveable { mutableStateOf(false) }

    Column(modifier = modifier) {
        Text(
            text = stringResource(R.string.s_family_stickers),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
        )
        ListItem(
            headlineContent = {
                Text(stringResource(R.string.s_family_stickers_count, items.size, ceiling.maxItems))
            },
            supportingContent = { Text(stringResource(R.string.s_family_stickers_explanation)) },
            leadingContent = { Icon(Icons.Outlined.EmojiEmotions, contentDescription = null) },
            trailingContent = {
                Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null)
            },
            modifier = Modifier
                .clickable(role = Role.Button) { managing = true }
                .testTag("family-stickers"),
        )
    }

    if (managing) {
        StickerNotices(viewModel)
        PackManagerDialog(viewModel = viewModel, maxItems = ceiling.maxItems, onDismiss = { managing = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PackManagerDialog(
    viewModel: StickerViewModel,
    maxItems: Int,
    onDismiss: () -> Unit,
) {
    val items by viewModel.items.collectAsStateWithLifecycle()
    val adding by viewModel.adding.collectAsStateWithLifecycle()
    // Saveable: the picker is another activity, and this one may be
    // recreated underneath it before the label has been asked for.
    var picked by rememberSaveable { mutableStateOf<android.net.Uri?>(null) }
    var removing by remember { mutableStateOf<PackItemEntity?>(null) }
    // Any picture: a `.webp` or `.png` is taken as it is, and anything else
    // is made into a sticker by fitting it whole into 512 x 512
    // (PackPicture). Asking for `image/*` rather than the two types is what
    // lets a photograph become one.
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) picked = uri
    }
    val full = items.size >= maxItems

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text(stringResource(R.string.s_family_stickers)) },
                    navigationIcon = {
                        IconButton(onClick = onDismiss) {
                            Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.s_close))
                        }
                    },
                )
            },
            floatingActionButton = {
                // Gone at the ceiling rather than greyed: the line below
                // says why, and an add that can only be refused is worse
                // than no button.
                if (!full) {
                    ExtendedFloatingActionButton(
                        onClick = { if (!adding) pick.launch("image/*") },
                        icon = { Icon(Icons.Filled.Add, contentDescription = null) },
                        text = { Text(stringResource(R.string.s_add_sticker)) },
                        modifier = Modifier.testTag("add-sticker"),
                    )
                }
            },
        ) { padding ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
            ) {
                Text(
                    text = if (full) {
                        stringResource(R.string.s_sticker_pack_full, maxItems)
                    } else {
                        stringResource(R.string.s_family_stickers_count, items.size, maxItems)
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
                if (items.isEmpty()) {
                    EmptyState(
                        icon = Icons.Outlined.EmojiEmotions,
                        title = stringResource(R.string.s_no_stickers_yet),
                        subtitle = stringResource(R.string.s_family_stickers_explanation),
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(MANAGER_CELL),
                        modifier = Modifier.fillMaxSize(),
                        // Room under the last row for the button over it.
                        contentPadding = PaddingValues(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 96.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(items, key = { it.id }) { item ->
                            ManagedSticker(
                                item = item,
                                load = viewModel::packFile,
                                // The author-or-owner rule decides whether
                                // the action is drawn at all; the server
                                // decides whether it happens.
                                onRemove = if (viewModel.canRemove(item)) ({ removing = item }) else null,
                            )
                        }
                    }
                }
            }
        }
    }

    picked?.let { uri ->
        StickerLabelDialog(
            onAdd = { label ->
                picked = null
                viewModel.addPicked(uri, label)
            },
            onDismiss = { picked = null },
        )
    }

    removing?.let { item ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text(stringResource(R.string.s_remove_sticker_q)) },
            text = { Text(stringResource(R.string.s_remove_sticker_explanation)) },
            confirmButton = {
                DestructiveTextButton(
                    label = stringResource(R.string.s_remove),
                    onClick = {
                        removing = null
                        viewModel.remove(item)
                    },
                )
            },
            dismissButton = {
                TextButton(onClick = { removing = null }) { Text(stringResource(R.string.s_cancel)) }
            },
        )
    }
}

@Composable
private fun ManagedSticker(
    item: PackItemEntity,
    load: suspend (PackItemEntity) -> java.io.File?,
    onRemove: (() -> Unit)?,
) {
    val picture = item.attachment
    val description = item.label ?: stringResource(R.string.s_sticker)
    val removeDescription = stringResource(R.string.s_remove_sticker)
    val attachments = LocalAttachments.current
    Box(modifier = Modifier.aspectRatio(1f)) {
        if (picture != null) {
            StickerImage(
                key = picture.id,
                load = { load(item) },
                contentDescription = description,
                modifier = Modifier
                    .fillMaxSize()
                    .padding(8.dp),
                retryKey = attachments?.retryToken,
                decodeBox = MANAGER_CELL * CELL_GROWTH,
            )
        }
        if (onRemove != null) {
            IconButton(
                onClick = onRemove,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .size(32.dp)
                    .testTag("remove-sticker-${item.id}"),
            ) {
                Icon(
                    imageVector = Icons.Filled.Close,
                    contentDescription = removeDescription,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier
                        .size(22.dp)
                        .background(MaterialTheme.colorScheme.surfaceContainerHighest, CircleShape)
                        .padding(3.dp),
                )
            }
        }
    }
}

/**
 * The optional words for a screen reader, asked once when a picture is
 * added. Fixed from then on: there is no edit, an item is its picture.
 */
@Composable
private fun StickerLabelDialog(
    onAdd: (String?) -> Unit,
    onDismiss: () -> Unit,
) {
    var label by rememberSaveable { mutableStateOf("") }
    // Counted the way the server counts — Unicode scalar values of the
    // trimmed text — and REFUSED in words when over, never cut as it is
    // typed: a label silently shortened is not the one somebody wrote, and
    // a cut by UTF-16 units can land in the middle of an emoji.
    val tooLong = !PackPicture.labelFits(label)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.s_add_sticker)) },
        text = {
            Column {
                OutlinedTextField(
                    value = label,
                    onValueChange = { label = it },
                    label = { Text(stringResource(R.string.s_sticker_label)) },
                    singleLine = true,
                    isError = tooLong,
                    supportingText = if (tooLong) {
                        {
                            Text(
                                stringResource(
                                    R.string.s_sticker_label_too_long,
                                    PackPicture.MAX_LABEL_CHARS,
                                ),
                            )
                        }
                    } else {
                        null
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("sticker-label"),
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.s_sticker_label_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Start,
                )
            }
        },
        confirmButton = {
            // Disabled while the label is over: the sentence under the field
            // says why, and no request is made to be refused.
            TextButton(
                onClick = { onAdd(label.trim().ifEmpty { null }) },
                enabled = !tooLong,
                modifier = Modifier.testTag("sticker-label-add"),
            ) {
                Text(stringResource(R.string.s_add_sticker))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_cancel)) }
        },
    )
}

/** A panel cell's smallest edge; the grid fits as many as the width allows. */
private val PANEL_CELL = 76.dp

/** The manager's cells are larger: there the pack is being looked at, not picked from. */
private val MANAGER_CELL = 104.dp

/**
 * How far past its minimum an adaptive grid's cell can grow: the grid adds
 * a column as soon as one more minimum fits, so from two columns up a cell
 * is under one and a half times it. What a cell's picture is DECODED for —
 * a cell that asked for the enlarged view's pixels would hold a 2000-pixel
 * sticker at sixteen megabytes, two dozen times over.
 */
private const val CELL_GROWTH = 1.5f
