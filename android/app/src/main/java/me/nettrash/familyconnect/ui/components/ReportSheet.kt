/*
 * ReportSheet.kt
 * Family Connect (Android)
 *
 * Reporting one member to the family's owner — from a message in the chat,
 * and from a row in the family roster.
 *
 * ONE sheet for both, deliberately. The disclosure it carries is a
 * protocol requirement rather than a nicety ("somebody who reports a
 * message without knowing that has been surprised by their own app"), and
 * two copies of a sentence like that drift apart exactly when it matters.
 */

package me.nettrash.familyconnect.ui.components

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R

/**
 * The four reasons a report may carry.
 *
 * The wire strings are the enum's own [wire] values and the protocol's
 * fixed list, so nine locales draw a row from string resources rather than
 * shipping untranslated prose to a moderator (docs/protocol.md,
 * "Reporting a member").
 */
internal enum class ReportReason(val wire: String, @StringRes val label: Int) {
    SPAM("spam", R.string.s_report_reason_spam),
    HARASSMENT("harassment", R.string.s_report_reason_harassment),
    INAPPROPRIATE("inappropriate", R.string.s_report_reason_inappropriate),
    OTHER("other", R.string.s_report_reason_other),
}

/**
 * Report one member to the family's owner.
 *
 * The disclosure is MANDATORY and not a footnote: the owner sees the
 * reporter's name and, when a message was named, its whole text frozen at
 * this moment. Somebody deciding whether to report has to know that before
 * they tap, not afterwards.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ReportSheet(
    displayName: String,
    /**
     * The operator's published contact, or null when unset. The honest
     * escalation path for the case this feature is weakest at: a report
     * ABOUT the owner never reaches the owner (docs/protocol.md,
     * "Reporting a member").
     */
    supportContact: String?,
    onDismiss: () -> Unit,
    onSubmit: (String) -> Unit,
) {
    var reason by remember { mutableStateOf(ReportReason.SPAM) }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp)
                .padding(bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = stringResource(R.string.s_report_member_title, displayName),
                style = MaterialTheme.typography.titleMedium,
            )
            Column(modifier = Modifier.selectableGroup()) {
                ReportReason.entries.forEach { choice ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .selectable(
                                selected = reason == choice,
                                role = Role.RadioButton,
                                onClick = { reason = choice },
                            )
                            .padding(vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        RadioButton(selected = reason == choice, onClick = null)
                        Text(
                            text = stringResource(choice.label),
                            style = MaterialTheme.typography.bodyLarge,
                        )
                    }
                }
            }
            Text(
                text = stringResource(R.string.s_report_disclosure),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (!supportContact.isNullOrEmpty()) {
                HorizontalDivider()
                Text(
                    text = stringResource(R.string.s_if_the_problem_is_the_owner),
                    style = MaterialTheme.typography.labelLarge,
                )
                // VERBATIM, selectable, never linkified: an operator may
                // write an address, a URL or a whole sentence, and three
                // apps guessing differently about which it is would be
                // worse than three apps showing the same text.
                SelectionContainer {
                    Text(text = supportContact, style = MaterialTheme.typography.bodyMedium)
                }
                Text(
                    text = stringResource(R.string.s_operator_published_contact),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.s_cancel))
                }
                TextButton(onClick = { onSubmit(reason.wire) }) {
                    Text(
                        text = stringResource(R.string.s_report),
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
    }
}
