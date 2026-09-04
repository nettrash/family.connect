/*
 * BoardScreen.kt
 * Family Connect (Android)
 *
 * The family board: a wall of sticker notes anyone can add to and rearrange
 * (docs/protocol.md, stringResource(R.string.s_board)).
 *
 * Positions are FRACTIONS of the board, not pixels, so the wall looks the
 * same on a phone and a tablet — this screen multiplies by its own size on
 * the way out and divides on the way in, and nothing but that conversion
 * knows about density.
 *
 * Two authorship rules, and the UI has to make both legible: anyone may
 * DRAG any note (tidying the wall is shared), but only the author may
 * rewrite or delete one. A note you cannot edit still opens — read-only,
 * saying who wrote it — rather than silently ignoring the tap.
 *
 * A note's SIZE is a step name, not a measurement (docs/protocol.md,
 * "Board"): the wire says "large" and this screen decides what large is
 * on a phone, the way it decides what "yellow" is. It sits with text and
 * colour as the author's call, so the drag path never touches it.
 *
 * iOS counterpart: ios/FamilyConnect/Views/BoardView.swift
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.automirrored.outlined.StickyNote2
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.components.isWideWindow
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.ui.components.EmptyState
import kotlin.math.roundToInt
import androidx.compose.ui.text.font.FontStyle
import me.nettrash.familyconnect.ui.chat.BlockedMessageRule

/** The six names the protocol allows. Unknown values fall back to yellow. */
object NoteColors {
    val palette = listOf("yellow", "pink", "blue", "green", "orange", "purple")

    fun compose(name: String): Color = when (name) {
        "pink" -> Color(0xFFFCC7D9)
        "blue" -> Color(0xFFC2E0FC)
        "green" -> Color(0xFFC9F0C9)
        "orange" -> Color(0xFFFFD9B3)
        "purple" -> Color(0xFFE0D1FA)
        else -> Color(0xFFFFF2B3)
    }

    /** The swatch's TalkBack label — the raw palette key is not display text. */
    fun label(name: String): Int = when (name) {
        "pink" -> R.string.s_color_pink
        "blue" -> R.string.s_color_blue
        "green" -> R.string.s_color_green
        "orange" -> R.string.s_color_orange
        "purple" -> R.string.s_color_purple
        else -> R.string.s_color_yellow
    }
}

/**
 * The three step names the protocol allows, and what each one IS on a
 * phone. Unknown values fall back to medium — the size every note had
 * before there was one — so a note a newer client wrote never fails to
 * draw here; it just draws the way it always did. The metrics live in
 * this one place so the sticker, its clamp and its text agree by
 * construction. (Phone idiom: iOS BoardView uses the same names with the
 * same shape; the Mac is wider, as everything is there.)
 */
object NoteSizes {
    const val SMALL = "small"
    const val MEDIUM = "medium"
    const val LARGE = "large"

    /** In the order the picker shows them. */
    val steps = listOf(SMALL, MEDIUM, LARGE)

    /** Collapses an unknown name to medium, so everything below has three cases. */
    fun resolve(name: String): String = if (name in steps) name else MEDIUM

    /** The sticker is square; this is its side. */
    fun side(name: String): Dp = when (resolve(name)) {
        SMALL -> 100.dp
        LARGE -> 220.dp
        else -> 132.dp
    }

    /** Lines of the text that show before it ellipsises. */
    fun maxLines(name: String): Int = when (resolve(name)) {
        SMALL -> 3
        LARGE -> 10
        else -> 5
    }

    /** Takes the theme's typography rather than reading it, so it stays plain Kotlin. */
    fun textStyle(name: String, typography: Typography): TextStyle = when (resolve(name)) {
        SMALL -> typography.bodySmall
        LARGE -> typography.bodyLarge
        else -> typography.bodyMedium
    }

    /** The picker's label — the raw step key is not display text. */
    fun label(name: String): Int = when (resolve(name)) {
        SMALL -> R.string.s_small
        LARGE -> R.string.s_large
        else -> R.string.s_medium
    }
}

/** A note being written or rewritten. `noteId` null = a new one. */
data class NoteDraft(
    val noteId: Long?,
    val text: String,
    val color: String,
    /**
     * The step name AS STORED — possibly one this client does not know,
     * kept raw so an untouched edit hands it back unchanged, the way
     * [color] is. A new note starts medium.
     */
    val size: String,
    val x: Double,
    val y: Double,
    val authorId: Long,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardScreen(
    onBack: () -> Unit,
    viewModel: BoardViewModel = hiltViewModel(),
) {
    val notes by viewModel.notes.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    var editing by remember { mutableStateOf<NoteDraft?>(null) }

    LaunchedEffect(Unit) { viewModel.refresh() }
    // The wall is in front of somebody, so everything on it has been shown
    // — including whatever lands WHILE they are looking, which is why this
    // keys on the notes and not on Unit. Marking only at the tap that opens
    // the board (ChatListScreen) marked an empty cache seen and then let the
    // whole wall, loaded a moment later, come back as unread (issue #53).
    LaunchedEffect(notes) { viewModel.markBoardSeen() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_board)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.s_back))
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = {
                // New notes land near the top-left, offset a little each
                // time so a burst of them does not stack into one
                // illegible pile.
                val slot = notes.size % NoteColors.palette.size
                editing = NoteDraft(
                    noteId = null,
                    text = "",
                    color = NoteColors.palette[slot],
                    size = NoteSizes.MEDIUM,
                    x = 0.12 + slot * 0.03,
                    y = 0.10 + slot * 0.06,
                    authorId = myUserId ?: -1L,
                )
            }) {
                Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.s_add_note))
            }
        },
    ) { padding ->
        BoxWithConstraints(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .background(MaterialTheme.colorScheme.surfaceContainerLowest),
        ) {
            val density = LocalDensity.current
            val boardWidthPx = with(density) { maxWidth.roundToPx() }
            val boardHeightPx = with(density) { maxHeight.roundToPx() }

            if (notes.isEmpty()) {
                EmptyState(
                    icon = Icons.AutoMirrored.Outlined.StickyNote2,
                    title = stringResource(R.string.s_the_board_is_empty),
                    subtitle = stringResource(R.string.s_add_a_note_everyone_in_the_family_sees_it),
                    modifier = Modifier.align(Alignment.Center),
                )
            }

            notes.forEach { note ->
                StickyNote(
                    note = note,
                    isHiddenByBlock = BlockedMessageRule.isNoteHidden(
                        note.authorId, myUserId ?: -1L, blockedUserIds,
                    ),
                    authorName = when (note.authorId) {
                        myUserId -> stringResource(R.string.s_you)
                        else -> memberNames[note.authorId]
                            ?: stringResource(R.string.s_someone)
                    },
                    boardWidthPx = boardWidthPx,
                    boardHeightPx = boardHeightPx,
                    onMoved = { x, y -> viewModel.moveNote(note.id, x, y) },
                    onTap = {
                        editing = NoteDraft(
                            noteId = note.id,
                            text = note.text,
                            color = note.color,
                            size = note.size,
                            x = note.x,
                            y = note.y,
                            authorId = note.authorId,
                        )
                    },
                )
            }
        }
    }

    editing?.let { draft ->
        NoteDialog(
            draft = draft,
            canEdit = draft.noteId == null || draft.authorId == myUserId,
            authorName = when (draft.authorId) {
                myUserId -> stringResource(R.string.s_you)
                else -> memberNames[draft.authorId] ?: stringResource(R.string.s_someone)
            },
            onDismiss = { editing = null },
            onSave = { text, color, size ->
                editing = null
                if (draft.noteId == null) {
                    viewModel.addNote(text, color, size, draft.x, draft.y)
                } else {
                    viewModel.editNote(draft.noteId, text, color, size)
                }
            },
            onDelete = draft.noteId?.let { id ->
                {
                    editing = null
                    viewModel.deleteNote(id)
                }
            },
        )
    }
}

/**
 * One sticker. Drag moves it locally at once and reports the FRACTION on
 * release — the server is told where it ended up, not every frame of how it
 * got there.
 *
 * The geometry runs in one direction: the stored fraction becomes a
 * top-left ORIGIN clamped to the board first, the drag delta is added to
 * THAT and clamped again, and what is drawn is what is reported back.
 * Adding the delta to the unclamped product instead meant a note stored
 * at x near 1 — drawn pinned to the edge — did not move until the finger
 * had travelled the whole overhang, which a large note makes a long way.
 *
 * The same dead travel came back by another door: the drag delta is only
 * zeroed when the server's answer lands, and a drop that ends on the very
 * fraction the note already has (pushed further into an edge it is
 * pinned to, or returned to its starting pixel) gets NO answer — the
 * server sees nothing changed and keeps the old row and seq. So a release
 * that would be a no-op is not sent at all, and the delta is zeroed here
 * instead; drawn equals origin in that case, so nothing jumps.
 */
@Composable
private fun StickyNote(
    note: NoteEntity,
    authorName: String,
    /**
     * Its author is blocked, so the note hides its CONTENT as well as its
     * author — the one object where a block takes the text too. A note is
     * a piece of writing pinned to a shared wall with no bubble to
     * collapse into a hidden row, so dropping only the name would hide
     * nothing that mattered (docs/protocol.md, "Board").
     *
     * The slot, size, colour and tilt all stay: a note the blocker hides
     * still occupies its slot, and the note ceiling is never projected per
     * reader.
     */
    isHiddenByBlock: Boolean,
    boardWidthPx: Int,
    boardHeightPx: Int,
    onMoved: (Double, Double) -> Unit,
    onTap: () -> Unit,
) {
    var dragX by remember(note.id) { mutableFloatStateOf(0f) }
    var dragY by remember(note.id) { mutableFloatStateOf(0f) }
    // A peek, not a setting: per note, per device, never on the wire and
    // never stored, and gone on the next launch. Keyed on the note so a
    // recomposition cannot carry a reveal onto a different one.
    var isRevealed by remember(note.id) { mutableStateOf(false) }
    val isHidden = isHiddenByBlock && !isRevealed
    val hiddenLabel = stringResource(R.string.s_hidden_blocked_member)
    // Resolved out here: a semantics block is not a composable context.
    // TalkBack gets the SAME masking the screen does — this label
    // concatenates the author AND the text, so masking only what is drawn
    // would have left both being read aloud.
    val noteDescription = if (isHidden) {
        stringResource(R.string.s_hidden_note_from_blocked)
    } else {
        stringResource(R.string.s_note_from, authorName, note.text)
    }

    // Reset the local offset when the AUTHORITATIVE position arrives.
    // Zeroing it on every drag-end instead would snap the note back to
    // where it started for the one frame before the server's answer
    // lands. The one release that gets no answer — a drop on the fraction
    // already stored — is zeroed in onDragEnd, where it is known.
    LaunchedEffect(note.x, note.y) {
        dragX = 0f
        dragY = 0f
    }

    // The STEP is the wire's name; the points grow with the wall. The
    // phone's 132dp medium is a stamp on a 10-inch tablet (iOS scales the
    // iPad's the same 1.45x).
    val side = NoteSizes.side(note.size) * (if (isWideWindow()) 1.45f else 1f)
    val sidePx = with(LocalDensity.current) { side.roundToPx() }
    val geometry = NoteGeometry(
        boardWidthPx = boardWidthPx,
        boardHeightPx = boardHeightPx,
        sidePx = sidePx,
        x = note.x,
        y = note.y,
    )
    // pointerInput restarts its block only when a KEY changes, so a plain
    // capture of the geometry would be frozen at the first composition's:
    // after one confirmed move the next drag-end would read the note's OLD
    // origin and report a stale fraction. The running block reads through
    // this holder instead, which always has the current frame's values.
    val latestGeometry by rememberUpdatedState(geometry)

    Box(
        modifier = Modifier
            // The offset draws it and drag-end reports it from the same
            // arithmetic, so the two cannot disagree.
            .offset { IntOffset(geometry.drawnX(dragX), geometry.drawnY(dragY)) }
            .size(side)
            .shadow(if (dragX == 0f && dragY == 0f) 2.dp else 8.dp, RoundedCornerShape(10.dp))
            .clip(RoundedCornerShape(10.dp))
            .background(NoteColors.compose(note.color))
            .pointerInput(note.id) {
                detectDragGestures(
                    onDrag = { change, delta ->
                        change.consume()
                        dragX += delta.x
                        dragY += delta.y
                    },
                    onDragEnd = {
                        // Read back from the DRAWN position, clamped, so a
                        // note dropped past the edge sticks to the edge —
                        // matching what the server does.
                        val g = latestGeometry
                        if (g.moved(dragX, dragY)) {
                            onMoved(g.fractionX(dragX), g.fractionY(dragY))
                        } else {
                            // Nothing to tell the server, and nothing it
                            // would answer with: the fraction is the one it
                            // holds, so the seq stays and the LaunchedEffect
                            // above never fires. Zero the delta here or it
                            // is carried into the next drag as dead travel.
                            dragX = 0f
                            dragY = 0f
                        }
                    },
                )
            }
            // The FIRST tap on a hidden note reveals it and does nothing
            // else. Falling through to `onTap` would open the note dialog,
            // which draws the very text the note is hiding.
            .pointerInput(note.id, isHidden) {
                detectTapGestures { if (isHidden) isRevealed = true else onTap() }
            }
            // A raw pointerInput publishes no click semantics, so without
            // this the note is invisible to TalkBack — the same trap the
            // link spans and the reply quote hit.
            .semantics {
                contentDescription = noteDescription
                role = Role.Button
            }
            .padding(10.dp),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            Text(
                text = if (isHidden) hiddenLabel else note.text,
                style = NoteSizes.textStyle(note.size, MaterialTheme.typography),
                color = Color.Black.copy(alpha = if (isHidden) 0.45f else 0.85f),
                fontStyle = if (isHidden) FontStyle.Italic else null,
                maxLines = NoteSizes.maxLines(note.size),
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            // The byline keeps its small style at every size: it is who
            // wrote the note, not part of what they wrote. While hidden
            // there is no byline at all — not an empty one, which would
            // still say a note came from somebody.
            if (!isHidden) {
                Text(
                    text = authorName,
                    style = MaterialTheme.typography.labelSmall,
                    color = Color.Black.copy(alpha = 0.5f),
                )
            }
        }
    }
}

/**
 * One sticker's place on one frame of the board, in pixels — see the
 * geometry note on [StickyNote]. The stored fraction is clamped into a
 * top-left ORIGIN first, the drag delta is added to that and clamped
 * again, and the fraction reported back is read off the drawn position.
 */
private class NoteGeometry(
    boardWidthPx: Int,
    boardHeightPx: Int,
    sidePx: Int,
    private val x: Double,
    private val y: Double,
) {
    private val width = boardWidthPx.coerceAtLeast(1)
    private val height = boardHeightPx.coerceAtLeast(1)
    private val maxX = (boardWidthPx - sidePx).coerceAtLeast(0)
    private val maxY = (boardHeightPx - sidePx).coerceAtLeast(0)
    private val originX = (x * boardWidthPx).roundToInt().coerceIn(0, maxX)
    private val originY = (y * boardHeightPx).roundToInt().coerceIn(0, maxY)

    fun drawnX(dragX: Float): Int = (originX + dragX).roundToInt().coerceIn(0, maxX)
    fun drawnY(dragY: Float): Int = (originY + dragY).roundToInt().coerceIn(0, maxY)

    fun fractionX(dragX: Float): Double = (drawnX(dragX).toDouble() / width).coerceIn(0.0, 1.0)
    fun fractionY(dragY: Float): Double = (drawnY(dragY).toDouble() / height).coerceIn(0.0, 1.0)

    /**
     * Whether a release here would change the STORED position — the same
     * comparison the server makes before it spends a seq on a move, so
     * the two agree on which drops are no-ops. Read off the reported
     * fraction rather than the pixel delta: a note pinned to an edge can
     * be dragged a long way and still land on the fraction it already has.
     */
    fun moved(dragX: Float, dragY: Float): Boolean =
        fractionX(dragX) != x || fractionY(dragY) != y
}

/**
 * The add/edit dialog. Read-only when the note is someone else's: the tap
 * still opens something rather than doing nothing, it just cannot be
 * changed. `onSave` hands back text, colour and size — the three things
 * that are the author's to decide.
 */
@Composable
internal fun NoteDialog(
    draft: NoteDraft,
    canEdit: Boolean,
    authorName: String,
    onDismiss: () -> Unit,
    onSave: (String, String, String) -> Unit,
    onDelete: (() -> Unit)?,
) {
    var text by remember(draft.noteId) { mutableStateOf(draft.text) }
    var color by remember(draft.noteId) { mutableStateOf(draft.color) }
    // Held RAW, like the colour: a name this client does not know is
    // drawn as medium, and the picker says so below, but Save must hand
    // back what was there unless the author actually picked a step —
    // otherwise an older client quietly downgrades what a newer server
    // accepted, just by opening the note to fix a typo.
    var size by remember(draft.noteId) { mutableStateOf(draft.size) }
    var confirmDelete by remember { mutableStateOf(false) }

    if (confirmDelete && onDelete != null) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.s_delete_this_note)) },
            confirmButton = {
                TextButton(onClick = onDelete) {
                    Text(stringResource(R.string.s_delete), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.s_cancel)) }
            },
        )
        return
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                stringResource(
                    if (draft.noteId == null) R.string.s_new_note else R.string.s_note,
                ),
            )
        },
        text = {
            Column {
                if (canEdit) {
                    OutlinedTextField(
                        value = text,
                        onValueChange = { text = it },
                        label = { Text(stringResource(R.string.s_note)) },
                        minLines = 3,
                        maxLines = 8,
                    )
                    Spacer(Modifier.size(16.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        NoteColors.palette.forEach { name ->
                            // Resolved out here: a semantics block is not a
                            // composable context.
                            val colorLabel = stringResource(NoteColors.label(name))
                            Box(
                                modifier = Modifier
                                    .size(30.dp)
                                    .clip(CircleShape)
                                    .background(NoteColors.compose(name))
                                    // clickable, not a raw detectTapGestures:
                                    // the same selection, plus ripple and the
                                    // minimum-touch-target hit expansion a
                                    // bare pointerInput never gets.
                                    .clickable { color = name }
                                    .semantics {
                                        contentDescription = colorLabel
                                        role = Role.Button
                                        selected = color == name
                                    },
                                contentAlignment = Alignment.Center,
                            ) {
                                if (color == name) {
                                    Box(
                                        modifier = Modifier
                                            .size(12.dp)
                                            .clip(CircleShape)
                                            .background(Color.Black.copy(alpha = 0.45f)),
                                    )
                                }
                            }
                        }
                    }
                    Spacer(Modifier.size(16.dp))
                    Text(
                        text = stringResource(R.string.s_size),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.size(4.dp))
                    // A segmented row rather than three swatches: a size
                    // has no colour to show, and the row already carries
                    // the single-choice semantics the swatches spell out
                    // by hand.
                    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                        NoteSizes.steps.forEachIndexed { index, name ->
                            SegmentedButton(
                                // Resolved for DISPLAY only: the selection
                                // shows the step the note draws as.
                                selected = NoteSizes.resolve(size) == name,
                                onClick = { size = name },
                                shape = SegmentedButtonDefaults.itemShape(
                                    index = index,
                                    count = NoteSizes.steps.size,
                                ),
                            ) {
                                Text(stringResource(NoteSizes.label(name)))
                            }
                        }
                    }
                } else {
                    Text(draft.text)
                    Spacer(Modifier.size(8.dp))
                    Text(
                        text = stringResource(R.string.s_written_by, authorName),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        confirmButton = {
            if (canEdit) {
                TextButton(
                    onClick = { onSave(text, color, size) },
                    enabled = text.isNotBlank(),
                ) {
                    Text(stringResource(R.string.s_save))
                }
            }
        },
        dismissButton = {
            Row {
                if (onDelete != null && canEdit) {
                    TextButton(onClick = { confirmDelete = true }) {
                        Text(stringResource(R.string.s_delete), color = MaterialTheme.colorScheme.error)
                    }
                    Spacer(Modifier.width(8.dp))
                }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_cancel)) }
            }
        },
    )
}
