/*
 * OpenPollsScreen.kt
 * Family Connect (Android)
 *
 * The family's open polls, on a screen of their own (docs/protocol.md,
 * "Finding the open ones").
 *
 * WHY THIS EXISTS. A poll is an ordinary message, which is what makes it
 * cheap — the question is the body, so previews, pushes, reply excerpts and
 * the assistant's transcript all read it with no new case between them. It is
 * also what loses it: a poll is drawn where it was sent, and once the family
 * has talked past it there is no way back but scrolling. A decision nobody
 * can find is a decision nobody makes.
 *
 * WHY A SCREEN AND NOT A BANNER. The thread is a reverseLayout LazyColumn
 * whose opening position and "N new messages" divider are decided ONCE at
 * open; a strip inserted above it changes the geometry that decision was made
 * against. And there is no cap on how many polls may be open, so a banner can
 * only ever show one of N.
 *
 * WHY IT REUSES PollBlock. Voting from here must be the same act as voting in
 * the chat — same bars, same faces, same "N of M voted", same refusal on a
 * closed poll. Drawing a second, simpler poll control here would be a second
 * place for those rules to drift.
 *
 * iOS counterpart: Views/OpenPollsView.swift
 */

package me.nettrash.familyconnect.ui.polls

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateSetOf
import androidx.compose.runtime.remember
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.foundation.clickable
import me.nettrash.familyconnect.ui.chat.BlockedMessageRule
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.ui.chat.PollBlock
import me.nettrash.familyconnect.ui.chat.buildPollView

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OpenPollsScreen(
    chatId: Long,
    onBack: () -> Unit,
    viewModel: OpenPollsViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val myUserId by viewModel.myUserId.collectAsStateWithLifecycle()
    val names by viewModel.names.collectAsStateWithLifecycle()
    val memberCount by viewModel.memberCount.collectAsStateWithLifecycle()
    val blocked by viewModel.blockedUserIds.collectAsStateWithLifecycle()
    val memberAvatars by viewModel.memberAvatars.collectAsStateWithLifecycle()
    // Rows this reader has peeked at: per row, per screen, never stored and
    // never on the wire — the thread's own reveal (protocol.md, "Blocking a
    // member").
    val revealed = remember { mutableStateSetOf<Long>() }

    LaunchedEffect(chatId) { viewModel.start(chatId) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_open_polls)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.s_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        when {
            state.loading && state.messages.isEmpty() ->
                Column(
                    modifier = Modifier.fillMaxSize().padding(padding),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    CircularProgressIndicator()
                }

            state.messages.isEmpty() ->
                Column(
                    modifier = Modifier.fillMaxSize().padding(padding).padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    // An error is said as one; an empty list is NOT a failure
                    // and is said as the good news it is — the family has
                    // decided everything.
                    Text(
                        text = state.error ?: stringResource(R.string.s_open_polls_none),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

            else ->
                LazyColumn(
                    modifier = Modifier.fillMaxSize().padding(padding),
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    // A refused vote is SAID, above the list it was refused
                    // on. Drawing the error only in the empty state left a
                    // poll closed under the reader sitting here refusing
                    // every tap with no word about why.
                    state.error?.let { error ->
                        item(key = "error") {
                            Text(
                                text = error,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                    }
                    items(state.messages, key = { it.id }) { message ->
                        val poll = message.poll ?: return@items
                        val me = myUserId ?: -1L
                        // A poll by somebody this reader has blocked is the
                        // same hidden row it is in the thread — placeholder
                        // and nothing else, one tap to peek (protocol.md,
                        // "Finding the open ones").
                        if (
                            BlockedMessageRule.isHidden(message.senderId, me, blocked) &&
                            message.id !in revealed
                        ) {
                            Text(
                                text = stringResource(R.string.s_hidden_blocked_member),
                                style = MaterialTheme.typography.bodyMedium,
                                fontStyle = FontStyle.Italic,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { revealed += message.id }
                                    .padding(vertical = 8.dp),
                            )
                            return@items
                        }
                        Column(
                            modifier = Modifier.fillMaxWidth(),
                            verticalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            // The QUESTION is the message body — not a field
                            // on the poll, which is the whole reason the
                            // endpoint answers with messages.
                            Text(
                                text = message.body,
                                style = MaterialTheme.typography.titleSmall,
                            )
                            Text(
                                text = names[message.senderId]
                                    ?: stringResource(R.string.s_someone),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            PollBlock(
                                poll = buildPollView(
                                    poll = poll,
                                    myUserId = me,
                                    names = names,
                                    familySize = memberCount,
                                    blockedUserIds = blocked,
                                ),
                                // The chat's faces, not initials: the same
                                // map ChatViewModel hands the thread.
                                memberAvatars = memberAvatars,
                                // Everything on this list is open by
                                // definition — the endpoint returns nothing
                                // else — so a tap always means something.
                                // The view model decides whether that tap is
                                // a vote or a retract, as the chat does.
                                canVote = true,
                                onVote = { optionId -> viewModel.vote(message.id, optionId) },
                                onDoubleTap = {},
                                onLongPress = {},
                            )
                            // Closing is the author's, and one-way — the
                            // same rule as the chat's long-press menu, drawn
                            // as a plain button here because a list of
                            // decisions is where somebody comes to finish
                            // one. Not offered to anybody else: the family
                            // owner does not outrank authorship.
                            if (message.senderId == me) {
                                TextButton(onClick = { viewModel.close(message.id) }) {
                                    Text(stringResource(R.string.s_close_poll))
                                }
                            }
                            HorizontalDivider()
                        }
                    }
                }
        }
    }
}
