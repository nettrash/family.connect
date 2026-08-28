/*
 * AttachmentAlbum.kt
 * Family Connect (Android)
 *
 * One message's photos and videos as a SET — what the bubble draws as a
 * stack of cards and what the full-screen viewer pages through.
 *
 * Deliberately pure: no composition, no bytes. The bubble reserves the
 * stack's shape from attachment metadata alone (docs/protocol.md carries
 * width/height on every photo and video), the same rule the single
 * thumbnail lives by — a shape decided by loaded pixels would resize the
 * bubble as each preview landed and shove the whole thread. The viewer's
 * start page is decided the same way, in the one place the whole message
 * is in scope (the per-row closure in ChatScreen), so the per-attachment
 * open callbacks below it never had to learn about albums.
 *
 * iOS counterpart: ios/FamilyConnect/Views/AttachmentAlbum.swift
 */

package me.nettrash.familyconnect.ui.components

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

/**
 * The media of one message, in the SENDER'S order, and which of them is
 * showing. [items] is never empty and holds media only — files, audio and
 * a location are rows in the bubble and have no page in the viewer.
 */
data class AttachmentAlbum(
    val items: List<AttachmentDto>,
    val index: Int,
) {
    init {
        require(items.isNotEmpty()) { "an album has at least one item" }
    }

    /** [index] pulled to the nearest end: a value can only ever point at a page that exists. */
    val clampedIndex: Int
        get() = index.coerceIn(0, items.lastIndex)

    /** The page showing. */
    val current: AttachmentDto
        get() = items[clampedIndex]

    companion object {
        /**
         * The card's shape limits, tighter than the single thumbnail's
         * (0.6…1.9): a stack of panoramas would be three slivers, and a
         * tall crop's peeking corners would climb above the balloon.
         */
        const val MIN_CARD_RATIO = 0.75f
        const val MAX_CARD_RATIO = 1.5f

        /** The photos and videos of [attachments], order kept. */
        fun media(attachments: List<AttachmentDto>): List<AttachmentDto> =
            attachments.filter { !it.isFile && !it.isAudio && !it.isLocation }

        /** Everything else — what the bubble draws as rows, order kept. */
        fun rows(attachments: List<AttachmentDto>): List<AttachmentDto> =
            attachments.filter { it.isFile || it.isAudio || it.isLocation }

        /**
         * The front card's shape, width over height, from the FIRST item's
         * declared dimensions (4:3 when the uploader could not say —
         * [AttachmentDto.aspectRatio]). A ratio rather than a size: the
         * card takes the bubble's media width, which is the balloon's to
         * decide (it shrinks with the window, the way the single thumbnail
         * does), and the height follows. Metadata only, on purpose; see
         * the file header.
         */
        fun cardRatio(first: AttachmentDto): Float =
            first.aspectRatio.coerceIn(MIN_CARD_RATIO, MAX_CARD_RATIO)

        /**
         * The album a tap on [tapped] opens: the whole message's media,
         * starting on the tapped item. A tapped item that is not among the
         * message's media (nothing sends one today) opens alone rather
         * than not at all.
         */
        fun opening(attachments: List<AttachmentDto>, tapped: AttachmentDto): AttachmentAlbum {
            val media = media(attachments)
            val index = media.indexOfFirst { it.id == tapped.id }
            return if (index < 0) AttachmentAlbum(listOf(tapped), 0) else AttachmentAlbum(media, index)
        }
    }
}
