package me.nettrash.familyconnect.ui.chat

/**
 * What the paperclip's menu holds, in what order, in which groups
 * (issue #78, docs/attachment-menu-2026-10-07.md).
 *
 * One menu on every client, read top to bottom, in three groups the
 * composer separates with a divider — never around a group that came out
 * empty:
 *
 * ```
 * Photo or video   — "Show the assistant a picture" INSTEAD, in the assistant chat
 * Camera           — opens "Take photo" / "Take video" in place of the menu
 * File
 * Paste
 * ──
 * Record voice message
 * Record video message
 * ──
 * Location
 * Poll             — the family chat only
 * ```
 *
 * The assistant chat takes images only, so its camera is a direct
 * "Take photo" rather than a "Camera" that would open a page of one item.
 *
 * Whether an item is ENABLED (a call, an unsent voice message, a busy
 * composer) is not decided here: the composer still owns that, exactly as
 * before. This is only which items exist, and where.
 *
 * A plain value with no Android in it, so the layout is pinned by an
 * ordinary unit test rather than inferred from a screenshot.
 */
object AttachMenu {

    enum class Item {
        PHOTO_OR_VIDEO,
        SHOW_ASSISTANT_PICTURE,
        /** Opens [cameraChoices] in place of the menu. */
        CAMERA,
        /**
         * A photo from the system camera: on the "Camera" page, and as the
         * assistant chat's own camera item, straight away.
         */
        TAKE_PHOTO,
        /** A video from the system camera: on the "Camera" page only. */
        TAKE_VIDEO,
        FILE,
        PASTE,
        RECORD_VOICE,
        RECORD_VIDEO,
        LOCATION,
        POLL,
    }

    /** What "Camera" opens, below its back row. */
    val cameraChoices: List<Item> = listOf(Item.TAKE_PHOTO, Item.TAKE_VIDEO)

    /**
     * The menu's groups, top to bottom, empty ones left out.
     *
     * @param assistantChat this is the member's own `ai` chat.
     * @param assistantPictures the assistant chat accepts pictures — this
     *   server can see and the family's owner allows it. Ignored elsewhere.
     * @param hasCamera the device has a camera to hand off to.
     * @param recordVoice "Record voice message" is offered (never in the
     *   assistant chat).
     * @param recordVideo round video is available here.
     * @param poll a poll can be started (the family chat).
     */
    fun groups(
        assistantChat: Boolean,
        assistantPictures: Boolean,
        hasCamera: Boolean,
        recordVoice: Boolean,
        recordVideo: Boolean,
        poll: Boolean,
    ): List<List<Item>> {
        val pictures = !assistantChat || assistantPictures
        val attach = buildList {
            when {
                !assistantChat -> add(Item.PHOTO_OR_VIDEO)
                assistantPictures -> add(Item.SHOW_ASSISTANT_PICTURE)
            }
            if (hasCamera && pictures) add(if (assistantChat) Item.TAKE_PHOTO else Item.CAMERA)
            add(Item.FILE)
            add(Item.PASTE)
        }
        val record = buildList {
            if (recordVoice && !assistantChat) add(Item.RECORD_VOICE)
            if (recordVideo && !assistantChat) add(Item.RECORD_VIDEO)
        }
        val share = buildList {
            add(Item.LOCATION)
            if (poll && !assistantChat) add(Item.POLL)
        }
        return listOf(attach, record, share).filter { it.isNotEmpty() }
    }
}
