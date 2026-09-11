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

import androidx.annotation.StringRes
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.background
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
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
import androidx.compose.material3.Switch
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DatePicker
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material.icons.filled.AddAPhoto
import androidx.compose.material.icons.filled.Event
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.layout.ContentScale
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
import java.time.Instant
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.NoteMentionsCodec
import me.nettrash.familyconnect.ui.chat.MentionSuggestionsRow
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.data.net.dto.RsvpCodec
import me.nettrash.familyconnect.ui.components.rememberAttachmentImage
import me.nettrash.familyconnect.ui.components.EmptyState
import kotlin.math.roundToInt
import androidx.compose.ui.text.font.FontFamily
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
/**
 * The cap on a note's text, from the protocol: "text is trimmed, non-empty
 * and at most 280 characters".
 *
 * Enforced where the author is TYPING. Before this the field was uncapped,
 * the save came back `validation`, and the view model discarded it — a note
 * over the cap simply never appeared, with nothing on screen to say why.
 *
 * iOS counterpart: NoteText in Views/NotePreview.swift.
 */
object NoteText {
    const val MAX_LENGTH = 280

    /**
     * The first 280 characters, counted the way the SERVER counts them.
     *
     * Rust's `chars().count()` is Unicode scalars; Kotlin's `String.length`
     * is UTF-16 units, which is two for every emoji and every character
     * outside the basic plane. Counting code points here means a note that
     * looks under the cap is never refused, and a family that writes in
     * emoji does not lose half its allowance.
     */
    fun capped(text: String): String {
        val points = text.codePointCount(0, text.length)
        if (points <= MAX_LENGTH) return text
        return text.substring(0, text.offsetByCodePoints(0, MAX_LENGTH))
    }

    /** How many more characters may be typed. Never negative. */
    fun remaining(text: String): Int =
        (MAX_LENGTH - text.codePointCount(0, text.length)).coerceAtLeast(0)

    /** Shown only once it starts to matter, so an ordinary note is written in peace. */
    fun shouldShowCounter(text: String): Boolean = remaining(text) <= 40
}

/**
 * The sticker as it will look, drawn inside the editor.
 *
 * The board's text FITS its note (docs/protocol.md, "Board"), which makes
 * the size step a choice with a visible result. The protocol asks for that
 * to be in front of the author while they write, rather than discovered on
 * the wall afterwards — and it is the answer to "the text should have an
 * impact on the note size" that does not take the size away from the author
 * or move everybody else's notes around.
 *
 * Drawn through the same NoteSizes the wall draws through, so the two
 * cannot drift. iOS counterpart: NotePreview in Views/NotePreview.swift.
 */
/**
 * The picture on a photo note.
 *
 * Through the same AttachmentRepository a message's photo comes from: the
 * bytes are cached once per device, and a board that fetched its own copies
 * would double the storage for the same pixels. The PREVIEW is what a
 * sticker wants — a 220.dp tile has no use for 1600 pixels, and the preview
 * is what arrives first on a slow connection.
 */
/**
 * The block an event note draws above its title: when, where, and how many
 * are coming — the count, not the names, because a sticker has room for the
 * news and the card that opens has room for the people.
 */
/**
 * Compose a new event: a title, when it starts, optionally when it ends,
 * and optionally where (docs/protocol.md, "Board").
 *
 * The pickers are Material 3's own — this app bundles no date library and
 * had no date picker anywhere before now. The wire wants RFC3339, so what
 * comes out of them is turned into an instant here rather than anywhere a
 * time zone could be lost.
 *
 * iOS counterpart: the event sections of NoteEditor in Views/BoardView.swift.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EventDialog(
    onDismiss: () -> Unit,
    onSave: (title: String, startsAt: String, endsAt: String?, place: String?) -> Unit,
) {
    var title by remember { mutableStateOf("") }
    var place by remember { mutableStateOf("") }
    // The next round hour: a family event is PLANNED, not stamped at the
    // instant somebody tapped a button.
    val opening = remember {
        ZonedDateTime.now().plusHours(1).withMinute(0).withSecond(0).withNano(0)
    }
    var startsAt by remember { mutableStateOf(opening) }
    var hasEnd by remember { mutableStateOf(false) }
    var endsAt by remember { mutableStateOf(opening.plusHours(1)) }
    var picking by remember { mutableStateOf<String?>(null) }

    if (picking != null) {
        val editingEnd = picking == "end"
        val current = if (editingEnd) endsAt else startsAt
        val dateState = rememberDatePickerState(
            initialSelectedDateMillis = current.toInstant().toEpochMilli(),
        )
        DatePickerDialog(
            onDismissRequest = { picking = null },
            confirmButton = {
                TextButton(onClick = {
                    dateState.selectedDateMillis?.let { millis ->
                        val picked = Instant.ofEpochMilli(millis).atZone(ZoneId.of("UTC"))
                        val moved = current
                            .withYear(picked.year)
                            .withMonth(picked.monthValue)
                            .withDayOfMonth(picked.dayOfMonth)
                        if (editingEnd) endsAt = moved else startsAt = moved
                        // An end before the start is what the server
                        // refuses; keep them in order here so nobody meets
                        // that refusal.
                        if (!editingEnd && endsAt.isBefore(moved)) endsAt = moved.plusHours(1)
                    }
                    picking = null
                }) { Text(stringResource(R.string.s_save)) }
            },
            dismissButton = {
                TextButton(onClick = { picking = null }) {
                    Text(stringResource(R.string.s_cancel))
                }
            },
        ) {
            DatePicker(state = dateState)
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.s_add_event)) },
        text = {
            Column {
                OutlinedTextField(
                    value = title,
                    onValueChange = { title = NoteText.capped(it) },
                    label = { Text(stringResource(R.string.s_event)) },
                    singleLine = true,
                )
                Spacer(Modifier.size(16.dp))
                Text(
                    text = stringResource(R.string.s_event_when),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                TextButton(onClick = { picking = "start" }) {
                    Text(
                        stringResource(R.string.s_event_starts) + ": " +
                            EventFormat.whenLine(startsAt.toInstant().toEpochMilli(), null),
                    )
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Switch(checked = hasEnd, onCheckedChange = { hasEnd = it })
                    Spacer(Modifier.size(8.dp))
                    Text(stringResource(R.string.s_event_has_end))
                }
                if (hasEnd) {
                    TextButton(onClick = { picking = "end" }) {
                        Text(
                            stringResource(R.string.s_event_ends) + ": " +
                                EventFormat.whenLine(endsAt.toInstant().toEpochMilli(), null),
                        )
                    }
                }
                Spacer(Modifier.size(8.dp))
                OutlinedTextField(
                    value = place,
                    onValueChange = { if (it.length <= 200) place = it },
                    label = { Text(stringResource(R.string.s_event_place)) },
                    singleLine = true,
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(
                        title.trim(),
                        startsAt.toInstant().toString(),
                        if (hasEnd) endsAt.toInstant().toString() else null,
                        place.trim().ifEmpty { null },
                    )
                },
                enabled = title.isNotBlank(),
            ) { Text(stringResource(R.string.s_save)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_cancel)) }
        },
    )
}

@Composable
internal fun NoteEventBlock(note: NoteEntity, modifier: Modifier = Modifier) {
    val startsAt = note.startsAt ?: return
    val rsvps = RsvpCodec.decode(note.rsvpsJson)
    val going = rsvps.count { it.answer == RsvpAnswers.GOING }
    val maybe = rsvps.count { it.answer == RsvpAnswers.MAYBE }
    val past = EventFormat.isPast(startsAt, note.endsAt, System.currentTimeMillis())
    Column(modifier = modifier) {
        Text(
            text = EventFormat.whenLine(startsAt, note.endsAt),
            style = MaterialTheme.typography.labelSmall,
            color = Color.Black.copy(alpha = if (past) 0.4f else 0.75f),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        if (!note.place.isNullOrEmpty()) {
            Text(
                text = note.place,
                style = MaterialTheme.typography.labelSmall,
                color = Color.Black.copy(alpha = 0.55f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (going > 0 || maybe > 0) {
            Text(
                text = when {
                    maybe == 0 -> stringResource(R.string.s_rsvp_going_count, going)
                    going == 0 -> stringResource(R.string.s_rsvp_maybe_count, maybe)
                    else -> stringResource(R.string.s_rsvp_going_maybe_count, going, maybe)
                },
                style = MaterialTheme.typography.labelSmall,
                color = Color.Black.copy(alpha = 0.55f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
internal fun NotePicture(attachment: AttachmentDto, modifier: Modifier = Modifier) {
    val bitmap = rememberAttachmentImage(attachment, preview = true)
    Box(
        modifier = modifier.clip(RoundedCornerShape(6.dp)).background(Color.Black.copy(alpha = 0.06f)),
        contentAlignment = Alignment.Center,
    ) {
        if (bitmap != null) {
            Image(
                bitmap = bitmap,
                // The picture is the note; the sticker's own semantics
                // already say what it is, so a second description here
                // would have TalkBack read everything twice.
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
        }
    }
}

@Composable
internal fun NotePreview(
    text: String,
    color: String,
    size: String,
    font: String,
    modifier: Modifier = Modifier,
) {
    val previewLabel = stringResource(R.string.s_note_preview)
    Box(
        modifier = modifier
            .size(NoteSizes.side(size))
            .clip(RoundedCornerShape(10.dp))
            .background(NoteColors.compose(color))
            .padding(10.dp)
            // One element, and never a second reading of the text the field
            // above already holds: what this adds is the LOOK.
            .semantics(mergeDescendants = true) { contentDescription = previewLabel },
    ) {
        Text(
            text = text,
            style = NoteSizes.textStyle(size, MaterialTheme.typography)
                .copy(fontFamily = NoteFonts.family(font)),
            color = Color.Black.copy(alpha = 0.85f),
            autoSize = NoteSizes.autoSize(size, MaterialTheme.typography),
            maxLines = NoteSizes.FITTED_MAX_LINES,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.fillMaxSize(),
        )
    }
}

/**
 * The four hands a note can be written in.
 *
 * A font is an INTENT, not a typeface (docs/protocol.md, "Board"): the wire
 * carries a name, and each client resolves it to a system face of its own,
 * exactly as `size` is a step and `color` a name. Android has no bundled
 * fonts and no downloadable ones — the theme says so in as many words
 * (ui/theme/Type.kt: "no custom font families, no downloadable fonts") —
 * and none are added here: all four are generic families the platform
 * already draws.
 *
 * `casual` is Cursive, the friendliest face this platform has to hand;
 * Apple draws the same intent with its rounded design. The two do not match
 * stroke for stroke, and are not meant to: a family choosing it is asking
 * for "not the plain one", which is a thing every platform can keep.
 *
 * iOS counterpart: NoteFont in Views/NoteFont.swift.
 */
/**
 * What a note IS: words on a sticker, or a picture pinned to the wall
 * (docs/protocol.md, "Board").
 *
 * A photo note is a note in every other respect — it takes a slot anyone
 * may move, counts against the same ceiling, rides the same feed and the
 * same seq, and a block hides it the same way. There is no second board,
 * because a photo on the family's wall is not a different wall.
 *
 * A kind this client has never heard of DRAWS AS TEXT rather than being
 * dropped: the note still has a slot on a shared wall, and a hole in the
 * family's layout is worse than a sticker that says only what it says —
 * which is what both predicates below give it, by asking for the kind they
 * know rather than excluding the one they do not.
 *
 * iOS counterpart: NoteKind in Views/NoteKind.swift.
 */
object NoteKinds {
    const val TEXT = "text"
    const val PHOTO = "photo"
    const val EVENT = "event"

    fun isPhoto(kind: String): Boolean = kind == PHOTO

    fun isEvent(kind: String): Boolean = kind == EVENT
}

/**
 * The three answers to an event, in the order a picker offers them
 * (docs/protocol.md, "Board").
 *
 * iOS counterpart: RsvpAnswer in Views/NoteEventCard.swift.
 */
object RsvpAnswers {
    const val GOING = "going"
    const val MAYBE = "maybe"
    const val NO = "no"

    val all = listOf(GOING, MAYBE, NO)

    @StringRes
    fun label(answer: String): Int = when (answer) {
        MAYBE -> R.string.s_rsvp_maybe
        NO -> R.string.s_rsvp_no
        else -> R.string.s_rsvp_going
    }
}

/**
 * When and where, formatted for the sticker.
 *
 * In the READER's locale and time zone, deliberately: the wire carries an
 * instant, and a family spread across two countries each sees the moment in
 * their own — which is the whole reason the protocol stores a timestamp
 * rather than a local time. iOS counterpart: EventFormat.
 */
object EventFormat {
    fun whenLine(startsAt: Long, endsAt: Long?): String {
        val zone = ZoneId.systemDefault()
        val starts = Instant.ofEpochMilli(startsAt).atZone(zone)
        val day = starts.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM))
        val from = starts.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
        if (endsAt == null) return "$day, $from"
        val ends = Instant.ofEpochMilli(endsAt).atZone(zone)
        val to = if (ends.toLocalDate() == starts.toLocalDate()) {
            ends.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
        } else {
            ends.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)) + " " +
                ends.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
        }
        return "$day, $from – $to"
    }

    /**
     * Has it already happened? A past event is drawn quieter rather than
     * removed: the wall is the family's, and clearing it is their call.
     */
    fun isPast(startsAt: Long, endsAt: Long?, now: Long): Boolean = (endsAt ?: startsAt) < now
}

object NoteFonts {
    const val PLAIN = "plain"
    const val SERIF = "serif"
    const val MONO = "mono"
    const val CASUAL = "casual"

    /** In the order the picker shows them, plainest first. */
    val hands = listOf(PLAIN, SERIF, MONO, CASUAL)

    /** Collapses an unknown name to plain, so everything below has four cases. */
    fun resolve(name: String): String = if (name in hands) name else PLAIN

    fun family(name: String): FontFamily = when (resolve(name)) {
        SERIF -> FontFamily.Serif
        MONO -> FontFamily.Monospace
        CASUAL -> FontFamily.Cursive
        else -> FontFamily.Default
    }

    @StringRes
    fun label(name: String): Int = when (resolve(name)) {
        SERIF -> R.string.s_font_serif
        MONO -> R.string.s_font_mono
        CASUAL -> R.string.s_font_casual
        else -> R.string.s_font_plain
    }
}

/**
 * A board note's text with the names it says drawn as names
 * (docs/protocol.md, "Board").
 *
 * BOLD, in the note's own ink, and never a colour of its own: a sticker's
 * pastel is a ground like any other, and a tint on it is the mention that
 * cannot be read. Whether a name is also a DOOR is the caller's — it is in
 * the note somebody has opened, and it is not on the sticker, whose whole
 * face is a drag handle.
 *
 * Web counterpart: `named_runs` in web/src/views/board.rs.
 * Apple counterpart: `MemberMentions.noteText` in Models/MemberMentions.swift.
 */
object NoteNames {

    private val HIGHLIGHT = SpanStyle(fontWeight = FontWeight.Bold)

    /** What the WALL draws: the names marked, and no door anywhere. */
    fun annotate(text: String, mentions: List<MentionDto>): AnnotatedString =
        build(text, mentions, doors = emptySet(), onOpen = {})

    /**
     * What an OPEN note draws: the same marks, and a door on every name in
     * [doors] — the members this reader could actually message.
     */
    fun reader(
        text: String,
        mentions: List<MentionDto>,
        doors: Set<Long>,
        onOpen: (Long) -> Unit,
    ): AnnotatedString = build(text, mentions, doors, onOpen)

    private fun build(
        text: String,
        mentions: List<MentionDto>,
        doors: Set<Long>,
        onOpen: (Long) -> Unit,
    ): AnnotatedString {
        if (mentions.isEmpty()) return AnnotatedString(text)
        val tokens = MemberMention.tokens(text, mentions)
        if (tokens.isEmpty()) return AnnotatedString(text)
        return buildAnnotatedString {
            var at = 0
            for ((range, member) in tokens) {
                // Walked in order, and a token that would step backwards is
                // dropped: the offsets index THIS string, and one applied
                // out of order would mark somebody else's words.
                if (range.first < at || range.last >= text.length) continue
                if (range.first > at) append(text.substring(at, range.first))
                val said = text.substring(range.first, range.last + 1)
                // The highlight is the same whether there is a door behind
                // it or not, and it is a SPAN rather than the link's own
                // `TextLinkStyles`: a name must read as a name on a pastel
                // even where nothing opens, and link styling is resolved
                // per state at draw time — one more thing that could take
                // the weight away.
                withStyle(HIGHLIGHT) {
                    if (member.userId in doors) {
                        // A real link annotation rather than a tap
                        // detector: this one gets the platform's own link
                        // semantics, so TalkBack finds the door without a
                        // hand-written custom action.
                        withLink(
                            LinkAnnotation.Clickable(
                                tag = MemberMention.url(member.userId),
                            ) { onOpen(member.userId) },
                        ) { append(said) }
                    } else {
                        append(said)
                    }
                }
                at = range.last + 1
            }
            if (at < text.length) append(text.substring(at))
        }
    }
}

/**
 * The wall's own look: a cork ground, and a pin through every note.
 *
 * Decoration, and nowhere on the wire (docs/protocol.md, "Board"): where a
 * pin sits is not a fact about the note, and a client that draws neither is
 * not wrong. The colours are fixed rather than taken from the theme — a
 * corkboard is a corkboard in both appearances, and the pastels on top are
 * fixed light colours too, which is why the ink on them is forced dark.
 *
 * Web counterpart: `.board-wall` and `.sticker::after` in web/styles.css.
 * Apple counterpart: `BoardGround` / `NotePin` in Views/NoteSize.swift.
 */
object BoardGround {
    private val cork = Color(0xFFCBB391)
    private val corkDark = Color(0xFF5B4A36)

    /** The ground: cork, lit from the top-left the way a wall in a room is. */
    fun brush(isDark: Boolean): Brush = Brush.linearGradient(
        0f to (if (isDark) corkDark else cork),
        0.55f to (if (isDark) corkDark else cork),
        1f to (if (isDark) Color(0xFF4C3D2C) else Color(0xFFBFA382)),
    )
}

/**
 * How the wall itself is sized (docs/protocol.md, "Board").
 *
 * The wall is TALLER than the window and it scrolls: a wall the size of the
 * window is a wall that fills up, and then a family has to take something
 * down before it can say anything. `x` and `y` stay fractions of the WALL,
 * so making it taller moves nothing relative to anything else.
 *
 * The factor is the same on all four clients even though the wire says
 * nothing about it — a note two thirds of the way down should be two thirds
 * of the way down on the phone and on the Mac.
 *
 * Web counterpart: `fc_text::board::WALL_SCREENS`.
 * Apple counterpart: `BoardWall` in Views/NoteSize.swift.
 */
object BoardWall {
    const val SCREENS = 1.6f

    /** Never shorter than the window, or fractions of the wall would sit
     *  behind its edges. */
    fun heightPx(visiblePx: Int): Int = maxOf((visiblePx * SCREENS).toInt(), visiblePx)
}

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

    /**
     * The lines the fitted text may take.
     *
     * Deliberately generous rather than a per-size count: fitting works by
     * making the type smaller, and a cap of five lines would stop it long
     * before the sticker was full. A backstop for a single unbroken word,
     * not a layout rule. iOS: NoteSize.fittedLineLimit.
     */
    const val FITTED_MAX_LINES = 20

    /**
     * How far the type may shrink before the text is cut instead.
     *
     * A FLOOR, because type small enough to be unreadable communicates no
     * better than an ellipsis. Expressed as a fraction of the size's own
     * type so the three steps shrink by the same proportion and track the
     * system font scale, rather than to one absolute sp that a large
     * accessibility setting would push ABOVE the ceiling — a floor above
     * the ceiling never fits anything. iOS: NoteSize.minimumTextScale.
     */
    const val MIN_TEXT_SCALE = 0.6f

    /**
     * The fitting range for one step: down from the size's own type to the
     * floor. `StepBased` treats text as fitting only when it is not
     * ellipsised, so the ellipsis in the sticker means "even the floor was
     * too big", which is exactly the protocol's rule.
     */
    fun autoSize(name: String, typography: Typography): TextAutoSize {
        val ceiling = textStyle(name, typography).fontSize
        return TextAutoSize.StepBased(
            minFontSize = ceiling * MIN_TEXT_SCALE,
            maxFontSize = ceiling,
        )
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
    /** The hand AS STORED, kept raw for the same reason as [size]. */
    val font: String,
    /** What this note IS — a photo and an event draw differently. */
    val kind: String = NoteKinds.TEXT,
    /** What this reader answered, on an event. Null when they have not. */
    val myAnswer: String? = null,
    /**
     * The members the note NAMES, as stored (docs/protocol.md, "Board").
     * Held so the opened note can draw them — the editor re-decides them
     * from the text on save, and never from this.
     */
    val mentions: List<MentionDto> = emptyList(),
    val x: Double,
    val y: Double,
    val authorId: Long,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardScreen(
    onBack: () -> Unit,
    /** Where a name in an open note leads (docs/protocol.md, "Board"). */
    onOpenChat: (Long) -> Unit = {},
    viewModel: BoardViewModel = hiltViewModel(),
) {
    val notes by viewModel.notes.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val blockedUserIds by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val memberNames by viewModel.memberNames.collectAsStateWithLifecycle()
    val mentionRoster by viewModel.mentionRoster.collectAsStateWithLifecycle()
    var editing by remember { mutableStateOf<NoteDraft?>(null) }

    LaunchedEffect(Unit) { viewModel.refresh() }
    val pinning by viewModel.pinning.collectAsStateWithLifecycle()
    var composingEvent by remember { mutableStateOf(false) }
    val pinFailed by viewModel.pinFailed.collectAsStateWithLifecycle()
    // The system photo picker: no permission, no READ_MEDIA_IMAGES, the
    // same one the composer uses. A wall pins PICTURES, so images only
    // (docs/protocol.md, "Board").
    val pickPhoto = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia(),
    ) { uri ->
        if (uri != null) viewModel.pinPhoto(uri, notes.size % NoteColors.palette.size)
    }
    if (pinFailed) {
        AlertDialog(
            onDismissRequest = viewModel::clearPinFailure,
            confirmButton = {
                TextButton(onClick = viewModel::clearPinFailure) {
                    Text(stringResource(R.string.s_dismiss))
                }
            },
            text = { Text(stringResource(R.string.s_pin_photo_failed)) },
        )
    }
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
            Column(horizontalAlignment = Alignment.End) {
            SmallFloatingActionButton(onClick = { composingEvent = true }) {
                Icon(
                    Icons.Filled.Event,
                    contentDescription = stringResource(R.string.s_add_event),
                )
            }
            Spacer(Modifier.size(12.dp))
            SmallFloatingActionButton(
                onClick = {
                    pickPhoto.launch(
                        PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly),
                    )
                },
            ) {
                if (pinning) {
                    // The upload can take a moment on a phone connection,
                    // and a button that looked idle would be tapped again.
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                } else {
                    Icon(
                        Icons.Filled.AddAPhoto,
                        contentDescription = stringResource(R.string.s_pin_photo),
                    )
                }
            }
            Spacer(Modifier.size(12.dp))
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
                    font = NoteFonts.PLAIN,
                    x = 0.12 + slot * 0.03,
                    y = 0.10 + slot * 0.06,
                    authorId = myUserId ?: -1L,
                )
            }) {
                Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.s_add_note))
            }
            }
        },
    ) { padding ->
        val darkGround = isSystemInDarkTheme()
        BoxWithConstraints(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                // Cork, not the theme's surface: a wall is a wall
                // (docs/protocol.md, "Board").
                .background(BoardGround.brush(darkGround)),
        ) {
            val density = LocalDensity.current
            val boardWidthPx = with(density) { maxWidth.roundToPx() }
            // THE WALL SCROLLS, because it is taller than the window
            // (docs/protocol.md, "Board"). The fractions are read against
            // the WALL, so nothing moves relative to anything else — the
            // bottom of it is simply below the fold.
            val visibleHeightPx = with(density) { maxHeight.roundToPx() }
            val boardHeightPx = BoardWall.heightPx(visibleHeightPx)
            val wallHeight = with(density) { boardHeightPx.toDp() }

            if (notes.isEmpty()) {
                EmptyState(
                    icon = Icons.AutoMirrored.Outlined.StickyNote2,
                    title = stringResource(R.string.s_the_board_is_empty),
                    subtitle = stringResource(R.string.s_add_a_note_everyone_in_the_family_sees_it),
                    modifier = Modifier.align(Alignment.Center),
                )
            }

            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(wallHeight)
                    .verticalScroll(rememberScrollState()),
            ) {
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
                            font = note.font,
                            kind = note.kind,
                            myAnswer = RsvpCodec.decode(note.rsvpsJson)
                                .firstOrNull { it.userId == myUserId }?.answer,
                            mentions = NoteMentionsCodec.decode(note.mentionsJson),
                            x = note.x,
                            y = note.y,
                            authorId = note.authorId,
                        )
                    },
                )
            }
            }
        }
    }

    if (composingEvent) {
        val slot = notes.size % NoteColors.palette.size
        EventDialog(
            onDismiss = { composingEvent = false },
            onSave = { title, startsAt, endsAt, place ->
                composingEvent = false
                viewModel.addEvent(
                    title = title,
                    color = "blue",
                    startsAt = startsAt,
                    endsAt = endsAt,
                    place = place,
                    x = 0.12 + slot * 0.03,
                    y = 0.10 + slot * 0.06,
                )
            },
        )
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
            onSave = { text, color, size, font ->
                editing = null
                if (draft.noteId == null) {
                    viewModel.addNote(text, color, size, font, draft.x, draft.y)
                } else {
                    viewModel.editNote(draft.noteId, text, color, size, font)
                }
            },
            onAnswer = { answer -> draft.noteId?.let { viewModel.answerEvent(it, answer) } },
            myAnswer = draft.myAnswer,
            roster = mentionRoster,
            onOpenChat = { userId ->
                // The note closes first: the chat it opens is what the
                // reader asked for, and a dialog still over it is not.
                editing = null
                viewModel.openDirectChat(userId, onOpenChat)
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
// internal, not private: the wall's own drawing is pinned by
// StickyNoteTest — see it for what must stay true of a sticker.
internal fun StickyNote(
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
    // A PHOTO WITH NO CAPTION IS THE BARE PICTURE (docs/protocol.md,
    // "Board"): no paper behind it, no padding around it and no author
    // line under it — a picture pinned to a wall. A caption brings the
    // card back, because the words need paper to sit on.
    val isBarePicture = !isHidden && NoteKinds.isPhoto(note.kind) && note.text.isBlank()
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
            .size(side),
    ) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .shadow(
                if (dragX == 0f && dragY == 0f) 2.dp else 8.dp,
                RoundedCornerShape(if (isBarePicture) 4.dp else 10.dp),
            )
            .clip(RoundedCornerShape(if (isBarePicture) 4.dp else 10.dp))
            .background(if (isBarePicture) Color.Transparent else NoteColors.compose(note.color))
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
            .padding(if (isBarePicture) 0.dp else 10.dp),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            // A pinned picture fills the sticker, with the caption under it
            // — and NOTHING while the note is hidden by a block: the
            // picture is content, exactly as the text is (protocol.md,
            // "Board").
            if (!isHidden && NoteKinds.isPhoto(note.kind)) {
                AttachmentsCodec.decode(note.attachmentJson)?.firstOrNull()?.let { picture ->
                    NotePicture(
                        attachment = picture,
                        modifier = if (isBarePicture) {
                            // Nothing else on the card: the picture IS the
                            // card.
                            Modifier.fillMaxSize()
                        } else {
                            Modifier
                                .fillMaxWidth()
                                .height(84.dp)
                        },
                    )
                    if (!isBarePicture) Spacer(Modifier.size(6.dp))
                }
            }
            // An event says WHEN before it says what: the date is the
            // reason it is on the wall (docs/protocol.md, "Board").
            if (!isHidden && NoteKinds.isEvent(note.kind) && note.startsAt != null) {
                NoteEventBlock(note = note)
                Spacer(Modifier.size(4.dp))
            }
            if (!isBarePicture) {
            // The names the note says, bold and in its own ink — and not
            // doors here: a sticker's face is a drag handle
            // (docs/protocol.md, "Board").
            //
            // Remembered, because this function re-runs on every frame of
            // a drag and decoding the note's names each time would spend a
            // gesture's worth of work on a string that cannot change while
            // the finger is down.
            val drawnText = remember(isHidden, hiddenLabel, note.text, note.mentionsJson) {
                if (isHidden) {
                    AnnotatedString(hiddenLabel)
                } else {
                    NoteNames.annotate(note.text, NoteMentionsCodec.decode(note.mentionsJson))
                }
            }
            Text(
                text = drawnText,
                style = NoteSizes.textStyle(note.size, MaterialTheme.typography)
                    // The hand the author chose (docs/protocol.md, "Board").
                    // A hidden note keeps it, like its colour and its slot:
                    // nothing about the SHAPE of a note is the blocked
                    // member's content.
                    .copy(fontFamily = NoteFonts.family(note.font)),
                color = Color.Black.copy(alpha = if (isHidden) 0.45f else 0.85f),
                fontStyle = if (isHidden) FontStyle.Italic else null,
                // The text FITS the sticker (docs/protocol.md, "Board"):
                // the type scales down from the size's own until the whole
                // note is inside it. `maxLines` is now a backstop for one
                // unbroken word, and the ellipsis only appears past the
                // floor, which is the one case a reader opens the note for.
                autoSize = NoteSizes.autoSize(note.size, MaterialTheme.typography),
                maxLines = NoteSizes.FITTED_MAX_LINES,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            }
            // The byline keeps its small style at every size: it is who
            // wrote the note, not part of what they wrote. While hidden
            // there is no byline at all — not an empty one, which would
            // still say a note came from somebody. Nor on a bare picture,
            // which has no paper under it to write one on.
            if (!isHidden && !isBarePicture) {
                Text(
                    text = authorName,
                    style = MaterialTheme.typography.labelSmall,
                    color = Color.Black.copy(alpha = 0.5f),
                )
            }
        }
    }
    // THE PIN, over the card's top edge — drawn in the outer box, not the
    // card, because the card is clipped and a pin inside it would either be
    // cut or take a line of the words. Decoration only: it takes no room
    // and no touches (docs/protocol.md, "Board").
    Box(
        modifier = Modifier
            .align(Alignment.TopCenter)
            .offset(y = (-4).dp)
            .size(10.dp)
            .clip(CircleShape)
            .background(
                Brush.radialGradient(
                    colors = listOf(Color(0xFFD8534C), Color(0xFF7A1A15)),
                    center = Offset(0.35f, 0.3f),
                ),
            ),
    )
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
    onSave: (String, String, String, String) -> Unit,
    /**
     * Say whether this reader is coming — ANY member may, so it is not part
     * of the save, which is the author's (docs/protocol.md, "Board").
     */
    onAnswer: (String?) -> Unit = {},
    myAnswer: String? = null,
    /**
     * Everybody a name may mean, and everybody a name may OPEN — the one
     * set, because they are the same question (docs/protocol.md, "Board").
     */
    roster: List<MentionDto> = emptyList(),
    onOpenChat: (Long) -> Unit = {},
    onDelete: (() -> Unit)?,
) {
    // A TextFieldValue, not a String: accepting a name off the strip
    // rewrites the tail, and the caret has to follow it to the end or the
    // next keystroke lands in the middle of the name just picked.
    var text by remember(draft.noteId) {
        mutableStateOf(TextFieldValue(draft.text, TextRange(draft.text.length)))
    }
    var color by remember(draft.noteId) { mutableStateOf(draft.color) }
    // Held RAW, like the colour: a name this client does not know is
    // drawn as medium, and the picker says so below, but Save must hand
    // back what was there unless the author actually picked a step —
    // otherwise an older client quietly downgrades what a newer server
    // accepted, just by opening the note to fix a typo.
    var size by remember(draft.noteId) { mutableStateOf(draft.size) }
    var font by remember(draft.noteId) { mutableStateOf(draft.font) }
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
                // Answering sits ABOVE the author's fields and outside the
                // canEdit gate: it is the one thing everybody may do here.
                if (NoteKinds.isEvent(draft.kind) && draft.noteId != null) {
                    Text(
                        text = stringResource(R.string.s_rsvp_question),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.size(4.dp))
                    var answered by remember(draft.noteId) { mutableStateOf(myAnswer) }
                    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                        val options = RsvpAnswers.all + listOf<String?>(null)
                        options.forEachIndexed { index, option ->
                            SegmentedButton(
                                selected = answered == option,
                                onClick = {
                                    answered = option
                                    onAnswer(option)
                                },
                                shape = SegmentedButtonDefaults.itemShape(index, options.size),
                            ) {
                                Text(
                                    stringResource(
                                        option?.let(RsvpAnswers::label) ?: R.string.s_rsvp_none,
                                    ),
                                )
                            }
                        }
                    }
                    Spacer(Modifier.size(16.dp))
                }
                if (canEdit) {
                    OutlinedTextField(
                        value = text,
                        // The cap lives where the typing is: a note the
                        // server would refuse never becomes a save that
                        // fails, which on this screen used to fail SILENTLY
                        // (docs/protocol.md, "Board").
                        onValueChange = { value ->
                            val capped = NoteText.capped(value.text)
                            // A cut moves the end, so the caret is put
                            // there rather than left past it.
                            text = if (capped == value.text) {
                                value
                            } else {
                                TextFieldValue(capped, TextRange(capped.length))
                            }
                        },
                        label = { Text(stringResource(R.string.s_note)) },
                        minLines = 3,
                        maxLines = 8,
                        supportingText = if (NoteText.shouldShowCounter(text.text)) {
                            {
                                Text(
                                    stringResource(
                                        R.string.s_note_characters_left,
                                        NoteText.remaining(text.text),
                                    ),
                                )
                            }
                        } else {
                            null
                        },
                        isError = NoteText.remaining(text.text) == 0,
                    )
                    // The names a half-typed `@` could mean
                    // (docs/protocol.md, "Board") — the chat composer's own
                    // strip, under the words being written.
                    val offered = MemberMention.query(text.text)
                        ?.let { query -> MemberMention.candidates(roster, query, excluding = emptySet()) }
                        .orEmpty()
                    if (offered.isNotEmpty()) {
                        MentionSuggestionsRow(
                            candidates = offered,
                            onPick = { name ->
                                val accepted = MemberMention.accept(text.text, name)
                                text = TextFieldValue(accepted, TextRange(accepted.length))
                            },
                        )
                    }
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
                    Spacer(Modifier.size(16.dp))
                    // The hand, with text, colour and size: all four are
                    // the author's (docs/protocol.md, "Board").
                    Text(
                        text = stringResource(R.string.s_font),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.size(4.dp))
                    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                        NoteFonts.hands.forEachIndexed { index, name ->
                            SegmentedButton(
                                selected = NoteFonts.resolve(font) == name,
                                onClick = { font = name },
                                shape = SegmentedButtonDefaults.itemShape(
                                    index = index,
                                    count = NoteFonts.hands.size,
                                ),
                            ) {
                                // Drawn IN the hand it names, so the choice
                                // shows what it does.
                                Text(
                                    text = stringResource(NoteFonts.label(name)),
                                    fontFamily = NoteFonts.family(name),
                                )
                            }
                        }
                    }
                    Spacer(Modifier.size(16.dp))
                    // The consequence, in front of the author: the same
                    // sticker the wall will draw, with the type already
                    // fitted (docs/protocol.md, "Board").
                    Box(
                        modifier = Modifier.fillMaxWidth(),
                        contentAlignment = Alignment.Center,
                    ) {
                        NotePreview(
                            text = text.text.ifEmpty { stringResource(R.string.s_your_note) },
                            color = color,
                            size = size,
                            font = font,
                        )
                    }
                } else {
                    // A name in an OPEN note is a door (docs/protocol.md,
                    // "Board"): bold on the sticker, tappable here — and
                    // only where there is somebody to open it with, which
                    // is what `roster` already answers.
                    Text(
                        NoteNames.reader(
                            text = draft.text,
                            mentions = draft.mentions,
                            doors = roster.map { it.userId }.toSet(),
                            onOpen = onOpenChat,
                        ),
                    )
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
                    onClick = { onSave(text.text, color, size, font) },
                    enabled = text.text.isNotBlank(),
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
