/*
 * MemberNames.kt
 * Family Connect (Android)
 *
 * One rule, in one place: what to CALL a member on screen.
 *
 * A deleted account keeps its row so the messages, notes and reactions it
 * left behind can still be attributed (docs/protocol.md, "Deleting an
 * account"), but its stored display name is the server's ENGLISH
 * placeholder — the protocol says so outright and asks a client that
 * understands the `deleted` flag to draw its own translation instead.
 * Every screen that resolves a name goes through here, so there is
 * exactly one place that can forget to.
 *
 * Resolved in the ViewModel rather than at the point of drawing because
 * that is where the name maps are built (see ChatViewModel.memberNames);
 * it is the same trade every localized message on this client makes —
 * see the @ApplicationContext note on SettingsViewModel.
 *
 * The Apple clients owe the same substitution — the placeholder is the
 * server's English either way.
 */

package me.nettrash.familyconnect.util

import android.content.Context
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.MemberEntity

/** What to draw for one member — their name, or the tombstone's. */
fun MemberEntity.resolvedDisplayName(context: Context): String =
    if (deleted) context.getString(R.string.s_deleted_account) else displayName

/**
 * userId → the name to draw, for a whole roster.
 *
 * The roster this is given is the FULL one (MemberDao.observeMembers),
 * tombstones included: a bubble from a deleted account still has to say
 * who wrote it.
 */
fun List<MemberEntity>.resolvedDisplayNames(context: Context): Map<Long, String> =
    associate { it.userId to it.resolvedDisplayName(context) }
