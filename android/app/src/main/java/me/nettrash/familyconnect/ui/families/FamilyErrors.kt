/*
 * FamilyErrors.kt
 * Family Connect (Android)
 *
 * The sentences for the errors several families added (#88;
 * docs/protocol.md, "Several families" and the error list). One map, so
 * the gate, the chat list's banner and the owner's screen say the same
 * thing for the same code.
 */

package me.nettrash.familyconnect.ui.families

import android.content.Context
import androidx.annotation.StringRes
import me.nettrash.familyconnect.R

object FamilyErrors {

    const val TOO_MANY_FAMILIES = "too_many_families"
    const val ALREADY_OWN_A_FAMILY = "already_own_a_family"
    const val MEMBER_FAMILIES_DISABLED = "member_families_disabled"
    const val MEMBER_IN_OTHER_FAMILIES = "member_in_other_families"
    const val INVITATION_NOT_PENDING = "invitation_not_pending"

    /** The sentence [code] reads as, or null when it is not one of these. */
    @StringRes
    fun messageRes(code: String?): Int? = when (code) {
        TOO_MANY_FAMILIES -> R.string.e_too_many_families
        ALREADY_OWN_A_FAMILY -> R.string.e_already_own_a_family
        MEMBER_FAMILIES_DISABLED -> R.string.e_member_families_disabled
        MEMBER_IN_OTHER_FAMILIES -> R.string.e_member_in_other_families
        INVITATION_NOT_PENDING -> R.string.e_invitation_gone
        else -> null
    }

    /**
     * The sentence itself. `too_many_families` names the cap —
     * [maxFamiliesPerAccount], from `GET /me`.
     */
    fun message(context: Context, code: String?, maxFamiliesPerAccount: Int): String? {
        val res = messageRes(code) ?: return null
        return if (code == TOO_MANY_FAMILIES) {
            context.getString(res, maxFamiliesPerAccount)
        } else {
            context.getString(res)
        }
    }
}
