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
 *
 * Since #72 it can also ask the SECOND question — whether the assistant
 * may send a short query it writes from the member's words to the lookup
 * providers (docs/protocol.md, "Consenting to the assistant", amended
 * 2026-10-03). One more line naming every provider, and a second way to
 * agree; only where the caller passes the providers AND a way to record
 * that answer, so a screen that cannot record it never offers it.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.chat.AssistantLookups

@Composable
fun AssistantConsentDialog(
    processor: String,
    familyHistory: Boolean,
    familyVision: Boolean,
    /**
     * Whether this server turns recordings into text on request
     * (`assistant.transcribe`): the disclosure then says the sound of one
     * goes to [processor] when its text is asked for (docs/protocol.md,
     * "Transcripts on request").
     */
    transcripts: Boolean = false,
    onAgree: () -> Unit,
    onDismiss: () -> Unit,
    /**
     * `assistant.lookups`: the providers a lookup would reach. Empty — the
     * default, and the answer on a server with no source — draws the screen
     * exactly as before.
     */
    lookupProviders: List<String> = emptyList(),
    /**
     * Records BOTH consents ("Agree With Lookups"), or the lookup consent
     * alone when [assistantAgreed]. Null means this screen cannot record
     * it, and then no lookup part is drawn.
     */
    onAgreeWithLookups: (() -> Unit)? = null,
    /**
     * This member already agreed to the assistant, and the screen is asked
     * only about lookups: "I Agree" then calls [onAgreeWithLookups].
     */
    assistantAgreed: Boolean = false,
) {
    val uriHandler = LocalUriHandler.current
    val providers = AssistantLookups.providers(lookupProviders)
    val buttons = AssistantLookups.consentButtons(
        lookupsOffered = providers.isNotEmpty() && onAgreeWithLookups != null,
        assistantAgreedAt = if (assistantAgreed) "agreed" else null,
    )
    val named = lookupProvidersPhrase(providers)
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
                if (transcripts) {
                    Text(stringResource(R.string.s_consent_transcript_sound_sent, processor))
                }
                // The lookup line, picked by `ai_history` like the
                // family-chat line above: with it off a mention takes only
                // itself, so no recent message can shape a query either.
                if (buttons != AssistantLookups.ConsentButtons.AGREE) {
                    Text(
                        text = stringResource(R.string.s_looking_things_up),
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        if (familyHistory) {
                            stringResource(R.string.s_consent_lookups_with_history, named)
                        } else {
                            stringResource(R.string.s_consent_lookups_alone, named)
                        },
                    )
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
            when (buttons) {
                AssistantLookups.ConsentButtons.AGREE ->
                    TextButton(onClick = onAgree) { Text(stringResource(R.string.s_i_agree)) }
                AssistantLookups.ConsentButtons.AGREE_TO_LOOKUPS ->
                    TextButton(onClick = { onAgreeWithLookups?.invoke() }) {
                        Text(stringResource(R.string.s_i_agree))
                    }
                // Two ways to agree, neither the default: the member picks
                // whether the second recipient is included.
                AssistantLookups.ConsentButtons.AGREE_WITH_OR_WITHOUT_LOOKUPS ->
                    Row(horizontalArrangement = Arrangement.End) {
                        Column(horizontalAlignment = Alignment.End) {
                            TextButton(onClick = { onAgreeWithLookups?.invoke() }) {
                                Text(stringResource(R.string.s_agree_with_lookups))
                            }
                            TextButton(onClick = onAgree) {
                                Text(stringResource(R.string.s_agree_without_lookups))
                            }
                        }
                    }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.s_not_now)) }
        },
    )
}

/**
 * The lookup providers as one phrase — "Brave Search, Open-Meteo and
 * Wikipedia" — with this language's own joiners ([AssistantLookups.joinProviders]).
 * Shared by the consent screen, the member's Settings row and the owner's
 * switch, so the three name the providers the same way.
 */
@Composable
fun lookupProvidersPhrase(providers: List<String>): String {
    val resources = LocalResources.current
    return AssistantLookups.joinProviders(
        providers,
        two = { a, b -> resources.getString(R.string.s_list_two, a, b) },
        three = { a, b, c -> resources.getString(R.string.s_list_three, a, b, c) },
    )
}
