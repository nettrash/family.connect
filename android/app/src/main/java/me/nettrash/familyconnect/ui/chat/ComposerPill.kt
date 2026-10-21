/*
 * ComposerPill.kt
 * Family Connect (Android)
 *
 * The composer's field as one filled pill (#106,
 * docs/composer-and-wide-layout-2026-10-20.md, 2.1): the text, and at its
 * trailing edge the controls that live INSIDE it — the stickers always,
 * the assistant chat's paintbrush, the empty field's video button. The
 * main composer and the thread's draw the same pill, so a reply in a
 * thread no longer looks like a second product (an outlined field and a
 * grey Send glyph, before).
 *
 * WHY A ROW AROUND A TRANSPARENT FIELD, and not the field's own
 * trailingIcon. Material's text field centres its trailing icon on the
 * field's whole height: fine for the video button, which only shows while
 * the field is one line, but the stickers now stay while typing, and a
 * sticker button floating in the middle of a five-line draft sits beside
 * nothing. In a Row aligned to the bottom, every icon stays level with the
 * LAST line — the one being written, beside the paperclip and Send — at one
 * line and at five alike. The pill (its colour and its shape) is the
 * Row's, the field inside it draws no container of its own.
 *
 * iOS counterpart: the composer capsule in ConversationView / ThreadView.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.EmojiEmotions
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.TextFieldColors
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R

/** The pill's corner: half its one-line height, so a one-line field is a capsule. */
private val PillShape = RoundedCornerShape(24.dp)

/**
 * The pill's one-line height: the 48dp target of the icons inside it and
 * of the paperclip beside it, so the whole row lines up at one line.
 */
internal val ComposerPillMinHeight: Dp = ComposerSlot.MIN_TARGET_ANDROID_DP.dp

/**
 * What levels the slot's 44dp disc with the 48dp pill and paperclip in a
 * bottom-aligned row: half the difference, above and below.
 */
internal val SLOT_LEVELLING: Dp = (ComposerPillMinHeight - 44.dp) / 2

/**
 * The pill: [field] (given `Modifier.weight(1f)` by its caller) and then
 * [trailing], bottom-aligned on the pill's colour.
 */
@Composable
internal fun ComposerPill(
    modifier: Modifier = Modifier,
    trailing: @Composable RowScope.() -> Unit = {},
    field: @Composable RowScope.() -> Unit,
) {
    Row(
        modifier = modifier
            .heightIn(min = ComposerPillMinHeight)
            .clip(PillShape)
            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
        verticalAlignment = Alignment.Bottom,
    ) {
        field()
        trailing()
    }
}

/**
 * The field inside the pill draws nothing of its own — no container, no
 * indicator line — in any state: the pill is the shape.
 */
@Composable
internal fun composerFieldColors(): TextFieldColors = TextFieldDefaults.colors(
    focusedContainerColor = Color.Transparent,
    unfocusedContainerColor = Color.Transparent,
    disabledContainerColor = Color.Transparent,
    errorContainerColor = Color.Transparent,
    focusedIndicatorColor = Color.Transparent,
    unfocusedIndicatorColor = Color.Transparent,
    disabledIndicatorColor = Color.Transparent,
    errorIndicatorColor = Color.Transparent,
)

/**
 * The text's padding inside the pill. 12dp above and below puts one line of
 * body text (24dp) at the pill's 48dp, centred on the icons beside it; the
 * end is tight when icons follow, whose own 48dp targets already leave
 * 12dp around their glyphs.
 */
internal fun composerFieldPadding(hasTrailing: Boolean): PaddingValues = PaddingValues(
    start = 16.dp,
    end = if (hasTrailing) 4.dp else 16.dp,
    top = 12.dp,
    bottom = 12.dp,
)

/**
 * The stickers, inside the field (#106): there whether the field is empty
 * or not, so nothing moves under the thumb, and a sticker is one tap away
 * wherever a message can be written. A tap opens the panel; a sticker is
 * SENT by the tap that picks it there, which is why it was never a line in
 * the paperclip's menu — those stage something for Send.
 */
@Composable
internal fun StickerFieldButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    FieldIconButton(enabled = enabled, onClick = onClick, modifier = modifier) {
        Icon(
            imageVector = Icons.Outlined.EmojiEmotions,
            contentDescription = stringResource(R.string.s_stickers),
        )
    }
}

/**
 * A control inside the pill: a 48dp target in the field's own secondary
 * colour, as the video button has drawn since #79 — the field's glyphs
 * read as part of the field, not as the row's buttons.
 */
@Composable
internal fun FieldIconButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    IconButton(
        onClick = onClick,
        enabled = enabled,
        colors = IconButtonDefaults.iconButtonColors(
            contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
        ),
        modifier = modifier.size(ComposerSlot.MIN_TARGET_ANDROID_DP.dp),
        content = content,
    )
}
