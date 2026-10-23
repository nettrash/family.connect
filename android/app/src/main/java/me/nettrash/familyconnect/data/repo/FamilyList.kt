/*
 * FamilyList.kt
 * Family Connect (Android)
 *
 * The rules of several families (#88; docs/protocol.md, "Several
 * families") that are decisions and not requests, held where a test can
 * reach them: which family the app shows, what the session status is, when
 * the switcher appears, when "New Family…" and "Join Another Family…" are
 * offered, which chats a family lists, how the switcher counts unread, and
 * who the owner's "Invite…" picker offers.
 *
 * Pure: no Android, no storage. SessionRepository, the chat list and the
 * owner's screen all ask here, so the six clients' one behaviour is spelled
 * once on this one.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.settings.SettingsState

object FamilyList {

    const val OWNER = "owner"

    /**
     * The family to show: the one stored while it is still in the list,
     * else the first — the home family, which `GET /me` puts first. Null
     * only for an account in no family at all.
     */
    fun current(families: List<FamilyDto>, stored: Long?): Long? =
        stored?.takeIf { id -> families.any { it.id == id } } ?: families.firstOrNull()?.id

    /**
     * The session status for the CURRENT family: its role, or — with no
     * family at all — today's gate states, unchanged (a pending request
     * waits, nothing at all creates or joins).
     */
    fun status(
        families: List<FamilyDto>,
        current: Long?,
        pending: List<PendingJoinRequestDto>,
    ): FamilyStatus {
        val family = families.firstOrNull { it.id == current }
        return when {
            family != null -> if (family.role == OWNER) FamilyStatus.OWNER else FamilyStatus.MEMBER
            pending.isNotEmpty() -> FamilyStatus.PENDING
            else -> FamilyStatus.NONE
        }
    }

    /**
     * Whether there is a switcher at all: two families or more, or one with
     * a pending invitation or join request to show. An account in ONE family
     * with nothing pending sees nothing of several families.
     */
    fun showsSwitcher(state: SettingsState): Boolean =
        state.families.size >= 2 ||
            (state.families.isNotEmpty() && (state.pendingJoinRequests.isNotEmpty() || state.invitations.isNotEmpty()))

    /** Under `max_families_per_account` — and on a server that has several families at all. */
    private fun underCap(state: SettingsState): Boolean =
        state.multiFamilyServer && state.maxFamiliesPerAccount > 0 && state.families.size < state.maxFamiliesPerAccount

    /**
     * "New Family…": the server lets members start families, this account
     * owns none of its families, and it is under the cap (docs/protocol.md,
     * `GET /me`). An account with no family at all creates its first from the
     * gate instead.
     */
    fun canStartAnother(state: SettingsState): Boolean =
        state.families.isNotEmpty() &&
            state.memberFamiliesEnabled &&
            state.families.none { it.role == OWNER } &&
            underCap(state)

    /** "Join Another Family…": while under the cap. */
    fun canJoinAnother(state: SettingsState): Boolean = state.families.isNotEmpty() && underCap(state)

    /**
     * Whether [chat] is listed in [familyId]. A chat that has not been told
     * its families — cached before them, or from a server that predates the
     * field, where "it means the one family the account has" — is the HOME
     * family's.
     */
    fun listedIn(chat: ChatEntity, familyId: Long?, homeFamilyId: Long?): Boolean {
        val ids = chat.familyIds ?: return familyId == homeFamilyId
        return familyId != null && familyId in ids
    }

    /** The chats [familyId] lists, in the order given. */
    fun chatsOf(chats: List<ChatEntity>, familyId: Long?, homeFamilyId: Long?): List<ChatEntity> =
        chats.filter { listedIn(it, familyId, homeFamilyId) }

    /**
     * The switcher's unread per family: the unread of the chats listed in
     * that family. A direct chat counts in EACH family that lists it.
     */
    fun unreadByFamily(chats: List<ChatEntity>, families: List<FamilyDto>): Map<Long, Int> {
        val home = families.firstOrNull()?.id
        return families.associate { family ->
            family.id to chatsOf(chats, family.id, home).sumOf { it.unreadCount.coerceAtLeast(0) }
        }
    }

    /**
     * The family a tap on a push about [chatFamilyIds]'s chat opens in
     * (docs/protocol.md, "Several families" — "Pushes"): the one the app is
     * showing when the chat is listed there, else the first family that
     * lists it. Null leaves the app where it is.
     */
    fun familyForChat(chatFamilyIds: List<Long>?, current: Long?, families: List<FamilyDto>): Long? {
        val ids = chatFamilyIds ?: return null
        if (current != null && current in ids) return current
        return families.firstOrNull { it.id in ids }?.id
    }

    /**
     * Who the owner's "Invite…" offers for [familyId]: the members of their
     * OTHER families who are not in this one — from the rosters this device
     * holds, never anybody else on the server — once each, by name.
     */
    fun inviteCandidates(
        memberships: List<MemberEntity>,
        familyId: Long,
        myUserId: Long?,
    ): List<MemberEntity> {
        val here = memberships.filter { it.familyId == familyId }.mapTo(HashSet()) { it.userId }
        return memberships
            .asSequence()
            .filter { it.familyId != familyId && !it.hasLeft && !it.deleted }
            .filter { it.userId != myUserId && it.userId !in here }
            .distinctBy { it.userId }
            .sortedBy { it.displayName.lowercase() }
            .toList()
    }

    /** Whether the owner holds another family to invite from — "Invite…" is hidden otherwise. */
    fun hasOtherFamily(families: List<FamilyDto>, familyId: Long): Boolean = families.any { it.id != familyId }
}
