/*
 * AssistantConsentDialog.kt
 * Family Connect (Android)
 *
 * Asking, once, before anything a member writes goes to the model
 * (docs/protocol.md, "Consenting to the assistant").
 *
 * Everything the person needs is ON THIS SCREEN and not only behind the
 * policy link: who receives the words, what travels with them, that the
 * answer lands in the chat for everyone in it to read, and that stopping
 * later cannot recall what has already been sent. The two family-chat
 * lines depend on the owner's own switches — with `ai_history` off a
 * mention takes nothing but itself, and a screen promising otherwise
 * would be asking permission for something that does not happen.
 *
 * It saves nothing itself: the buttons hand the answer back, and the
 * caller writes it through the repository that holds the server's stamp.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R

@Composable
fun AssistantConsentDialog(
    processor: String,
    familyHistory: Boolean,
    familyVision: Boolean,
    onAgree: () -> Unit,
    onDismiss: () -> Unit,
) {
    val uriHandler = LocalUriHandler.current
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.s_the_assistant)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = stringResource(R.string.s_before_the_assistant_answers),
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(stringResource(R.string.s_consent_words_leave_for_processor, processor))
                Text(
                    if (familyHistory) {
                        stringResource(R.string.s_consent_family_history_travels)
                    } else {
                        stringResource(R.string.s_consent_family_mention_alone)
                    },
                )
                if (familyVision) {
                    Text(stringResource(R.string.s_consent_photo_only_when_attached))
                }
                Text(stringResource(R.string.s_consent_answer_lands_in_the_chat))
                Text(stringResource(R.string.s_consent_can_stop_in_settings))
                // The policy is still linked, because the guideline asks
                // for both: the disclosure where the answer is given, and
                // a policy that holds the same promises.
                TextButton(
                    onClick = {
                        uriHandler.openUri("https://nettrash.me/play/familyconnect/privacy.html")
                    },
                ) {
                    Text(stringResource(R.string.s_privacy_policy))
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onAgree) { Text(stringResource(R.string.s_i_agree)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_not_now)) }
        },
    )
}
