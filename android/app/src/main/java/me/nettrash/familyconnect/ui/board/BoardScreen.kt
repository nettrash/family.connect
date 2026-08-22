/*
 * BoardScreen.kt
 * Family Connect (Android)
 *
 * The family board: a wall of sticker notes anyone can add to and rearrange
 * (docs/protocol.md, "Board").
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
 * iOS counterpart: ios/FamilyConnect/Views/BoardView.swift
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.outlined.StickyNote2
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.ui.components.EmptyState
import kotlin.math.roundToInt

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
}

/** A note being written or rewritten. `noteId` null = a new one. */
data class NoteDraft(
    val noteId: Long?,
    val text: String,
    val color: String,
    val x: Double,
    val y: Double,
    val authorId: Long,
)

private val NOTE_SIDE = 132.dp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardScreen(
    onBack: () -> Unit,
    viewModel: BoardViewModel = hiltViewModel(),
) {
    val notes by viewModel.notes.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    var editing by remember { mutableStateOf<NoteDraft?>(null) }

    LaunchedEffect(Unit) { viewModel.refresh() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Board") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
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
                    x = 0.12 + slot * 0.03,
                    y = 0.10 + slot * 0.06,
                    authorId = myUserId ?: -1L,
                )
            }) {
                Icon(Icons.Filled.Add, contentDescription = "Add note")
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
            val sidePx = with(density) { NOTE_SIDE.roundToPx() }

            if (notes.isEmpty()) {
                EmptyState(
                    icon = Icons.Outlined.StickyNote2,
                    title = "The board is empty",
                    subtitle = "Add a note — everyone in the family sees it.",
                    modifier = Modifier.align(Alignment.Center),
                )
            }

            notes.forEach { note ->
                StickyNote(
                    note = note,
                    authorName = when (note.authorId) {
                        myUserId -> "You"
                        else -> memberNames[note.authorId] ?: "Someone"
                    },
                    boardWidthPx = boardWidthPx,
                    boardHeightPx = boardHeightPx,
                    sidePx = sidePx,
                    onMoved = { x, y -> viewModel.moveNote(note.id, x, y) },
                    onTap = {
                        editing = NoteDraft(
                            noteId = note.id,
                            text = note.text,
                            color = note.color,
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
                myUserId -> "You"
                else -> memberNames[draft.authorId] ?: "Someone"
            },
            onDismiss = { editing = null },
            onSave = { text, color ->
                editing = null
                if (draft.noteId == null) {
                    viewModel.addNote(text, color, draft.x, draft.y)
                } else {
                    viewModel.editNote(draft.noteId, text, color)
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
 */
@Composable
private fun StickyNote(
    note: NoteEntity,
    authorName: String,
    boardWidthPx: Int,
    boardHeightPx: Int,
    sidePx: Int,
    onMoved: (Double, Double) -> Unit,
    onTap: () -> Unit,
) {
    var dragX by remember(note.id) { mutableFloatStateOf(0f) }
    var dragY by remember(note.id) { mutableFloatStateOf(0f) }

    // Reset the local offset only when the AUTHORITATIVE position arrives.
    // Zeroing it on drag-end instead would snap the note back to where it
    // started for the one frame before the server's answer lands.
    LaunchedEffect(note.x, note.y) {
        dragX = 0f
        dragY = 0f
    }

    val maxX = (boardWidthPx - sidePx).coerceAtLeast(0)
    val maxY = (boardHeightPx - sidePx).coerceAtLeast(0)

    Box(
        modifier = Modifier
            .offset {
                IntOffset(
                    ((note.x * boardWidthPx).toFloat() + dragX).roundToInt().coerceIn(0, maxX),
                    ((note.y * boardHeightPx).toFloat() + dragY).roundToInt().coerceIn(0, maxY),
                )
            }
            .size(NOTE_SIDE)
            .shadow(if (dragX == 0f && dragY == 0f) 2.dp else 8.dp, RoundedCornerShape(10.dp))
            .clip(RoundedCornerShape(10.dp))
            .background(NoteColors.compose(note.color))
            .pointerInput(note.id, boardWidthPx, boardHeightPx) {
                detectDragGestures(
                    onDrag = { change, delta ->
                        change.consume()
                        dragX += delta.x
                        dragY += delta.y
                    },
                    onDragEnd = {
                        // Clamped, so a note dropped past the edge sticks
                        // to the edge — matching what the server does.
                        val width = boardWidthPx.coerceAtLeast(1)
                        val height = boardHeightPx.coerceAtLeast(1)
                        val x = ((note.x * width + dragX) / width).coerceIn(0.0, 1.0)
                        val y = ((note.y * height + dragY) / height).coerceIn(0.0, 1.0)
                        onMoved(x, y)
                    },
                )
            }
            .pointerInput(note.id) { detectTapGestures { onTap() } }
            // A raw pointerInput publishes no click semantics, so without
            // this the note is invisible to TalkBack — the same trap the
            // link spans and the reply quote hit.
            .semantics {
                contentDescription = "Note from $authorName: ${note.text}"
                role = Role.Button
            }
            .padding(10.dp),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            Text(
                text = note.text,
                style = MaterialTheme.typography.bodyMedium,
                color = Color.Black.copy(alpha = 0.85f),
                maxLines = 5,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = authorName,
                style = MaterialTheme.typography.labelSmall,
                color = Color.Black.copy(alpha = 0.5f),
            )
        }
    }
}

/**
 * The add/edit dialog. Read-only when the note is someone else's: the tap
 * still opens something rather than doing nothing, it just cannot be
 * changed.
 */
@Composable
private fun NoteDialog(
    draft: NoteDraft,
    canEdit: Boolean,
    authorName: String,
    onDismiss: () -> Unit,
    onSave: (String, String) -> Unit,
    onDelete: (() -> Unit)?,
) {
    var text by remember(draft.noteId) { mutableStateOf(draft.text) }
    var color by remember(draft.noteId) { mutableStateOf(draft.color) }
    var confirmDelete by remember { mutableStateOf(false) }

    if (confirmDelete && onDelete != null) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("Delete this note?") },
            confirmButton = {
                TextButton(onClick = onDelete) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text("Cancel") }
            },
        )
        return
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (draft.noteId == null) "New note" else "Note") },
        text = {
            Column {
                if (canEdit) {
                    OutlinedTextField(
                        value = text,
                        onValueChange = { text = it },
                        label = { Text("Note") },
                        minLines = 3,
                        maxLines = 8,
                    )
                    Spacer(Modifier.size(16.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        NoteColors.palette.forEach { name ->
                            Box(
                                modifier = Modifier
                                    .size(30.dp)
                                    .clip(CircleShape)
                                    .background(NoteColors.compose(name))
                                    .pointerInput(name) { detectTapGestures { color = name } }
                                    .semantics {
                                        contentDescription = name
                                        role = Role.Button
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
                } else {
                    Text(draft.text)
                    Spacer(Modifier.size(8.dp))
                    Text(
                        text = "Written by $authorName",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        confirmButton = {
            if (canEdit) {
                TextButton(
                    onClick = { onSave(text, color) },
                    enabled = text.isNotBlank(),
                ) {
                    Text("Save")
                }
            }
        },
        dismissButton = {
            Row {
                if (onDelete != null && canEdit) {
                    TextButton(onClick = { confirmDelete = true }) {
                        Text("Delete", color = MaterialTheme.colorScheme.error)
                    }
                    Spacer(Modifier.width(8.dp))
                }
                TextButton(onClick = onDismiss) { Text("Cancel") }
            }
        },
    )
}
