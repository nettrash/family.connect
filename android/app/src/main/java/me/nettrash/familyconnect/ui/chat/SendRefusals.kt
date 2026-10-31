/*
 * SendRefusals.kt
 * Family Connect (Android)
 *
 * What the "Message not sent" dialog says about a send the server refused
 * for good, when this app has words for the refusal.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.annotation.StringRes
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.repo.PaidFamilies

object SendRefusals {

    /**
     * The sentence for a refusal [code], or null to keep the dialog's usual
     * "try again or delete" — every code this app has no words of its own
     * for, and a send that simply ran out of attempts.
     *
     * `storage_allowance_used`: the upload was for a PAID family past its
     * storage (docs/protocol.md, "Paid families"). Terminal — it is never
     * retried by itself — and not the server's disk being full, which is a
     * different refusal with a different fix.
     */
    @StringRes
    fun sentence(code: String?): Int? = when (code) {
        PaidFamilies.STORAGE_ALLOWANCE_USED -> R.string.e_storage_allowance_used
        else -> null
    }
}
