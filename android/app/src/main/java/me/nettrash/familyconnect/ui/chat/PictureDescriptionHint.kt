package me.nettrash.familyconnect.ui.chat

/**
 * When to say, beside a picture request, that real names and brands are
 * often refused (docs/protocol.md, "Pictures").
 *
 * The provider's own filter refuses a description that names a real
 * person, a public figure, a brand or a trademarked character, and the
 * server now tries ONE rewrite before giving up. Neither is visible from
 * a composer, so the courtesy is the one the picture strips pay: say it
 * at the moment it matters, where the member can still act on it — while
 * they are describing the picture, not after a refusal has cost them the
 * wait.
 *
 * A plain value with no Android in it, so the rule is pinned by an
 * ordinary unit test rather than inferred from a screenshot.
 */
object PictureDescriptionHint {

    /**
     * Does the COMPOSER show the hint?
     *
     * @param draft the composer's text as typed.
     * @param picturesOffered whether this composer offers `/draw` at all —
     *   the same answer that shows the "ask for a picture" button, so on a
     *   server with no images deployment, or in a chat the assistant is
     *   not in, `/draw` is just text and there is nothing to warn about.
     * @param inFamilyChat whether this is the family chat, where the
     *   server only reads a picture request off a message that also
     *   mentions the assistant: `/draw a cat` with no `@ai` in it is an
     *   ordinary message there, and a hint about how it will be drawn
     *   would be a promise that it will be.
     * @param editing whether the composer is borrowed to rewrite an old
     *   message — which calls no model, so nothing is being drawn.
     */
    fun inComposer(
        draft: String,
        picturesOffered: Boolean,
        inFamilyChat: Boolean,
        editing: Boolean,
    ): Boolean {
        if (!picturesOffered || editing) return false
        if (!AssistantMention.isStartingPicture(draft)) return false
        // Anywhere in the body, not only the leading one the grammar may
        // have skipped: the server routes a family-chat message to the
        // assistant on ANY mention (handlers_chat.rs, `model_surface`) and
        // then reads the token by the grammar alone, so `/draw a cat @ai`
        // is drawn — with `@ai` as part of the description.
        return !inFamilyChat || AssistantMention.mentions(draft)
    }

    /**
     * Does the BOARD's event dialog show the hint beside "Draw a
     * backdrop"? Exactly when that control is there: the event's title
     * is the description (nothing else leaves the server), so the hint
     * belongs wherever the
     * member can ask for one to be drawn — and nowhere the button is
     * absent, because then nothing will be.
     *
     * @param canEdit whether this reader is the event's author (a
     *   backdrop is the author's to ask for).
     * @param canDraw whether this server can draw at all.
     */
    fun onBackdrop(canEdit: Boolean, canDraw: Boolean): Boolean = canEdit && canDraw
}
