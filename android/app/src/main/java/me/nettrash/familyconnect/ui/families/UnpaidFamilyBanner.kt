/*
 * UnpaidFamilyBanner.kt
 * Family Connect (Android)
 *
 * The banner over a family's chat list while it is unpaid (#89;
 * docs/protocol.md, "Paid families"): the family will be deleted on that
 * day because its subscription has ended. A member is told who the owner
 * is — the one person who can renew it; the owner is told only that it
 * cannot be renewed from this app, which names no other app and no store
 * (Google Play's payments policy). It sits with the invitation banner,
 * the chat list's other family-level notice, and goes when `delete_at`
 * goes.
 */

package me.nettrash.familyconnect.ui.families

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import java.time.ZoneId
import java.util.Locale
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.repo.PaidFamilies
import me.nettrash.familyconnect.util.TimeFormat

/** One sentence and its arguments, resolved where it is drawn. */
data class BannerSentence(@param:StringRes val res: Int, val args: List<String>)

/**
 * The banner's words: the owner's sentence or the member's, with the
 * deadline as a day in the reader's own time zone and words.
 */
fun unpaidBannerSentence(
    banner: PaidFamilies.UnpaidBanner,
    nowMillis: Long = System.currentTimeMillis(),
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
): BannerSentence {
    val date = TimeFormat.calendarDay(banner.deleteAtMillis, nowMillis = nowMillis, zone = zone, locale = locale)
    return if (banner.isOwner) {
        BannerSentence(R.string.s_banner_owner_not_here, listOf(banner.familyName, date))
    } else {
        BannerSentence(R.string.s_banner_member, listOf(banner.familyName, date, banner.ownerName.orEmpty()))
    }
}

@Composable
fun UnpaidFamilyBanner(
    banner: PaidFamilies.UnpaidBanner,
    modifier: Modifier = Modifier,
) {
    val sentence = unpaidBannerSentence(banner)
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.errorContainer),
    ) {
        Row(modifier = Modifier.padding(16.dp), verticalAlignment = Alignment.Top) {
            Icon(
                Icons.Outlined.Schedule,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.onErrorContainer,
            )
            Spacer(Modifier.width(12.dp))
            Text(
                text = stringResource(sentence.res, *sentence.args.toTypedArray()),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
        }
    }
}
