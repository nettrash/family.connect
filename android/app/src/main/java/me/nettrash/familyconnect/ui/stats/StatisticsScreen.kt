/*
 * StatisticsScreen.kt
 * Family Connect (Android)
 *
 * What the family has actually sent (protocol.md, "Family statistics").
 * Every member sees the same numbers — it is a shared curiosity, not an
 * owner's dashboard.
 *
 * Nothing is cached: this is a page opened occasionally, and a stale count
 * would be worse than a moment's spinner.
 *
 * iOS/macOS counterpart: ios/FamilyConnect/Views/StatisticsView.swift
 */

package me.nettrash.familyconnect.ui.stats

import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import me.nettrash.familyconnect.ui.components.readableColumn
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AiStatsDto
import me.nettrash.familyconnect.data.net.dto.MemberStatsDto
import me.nettrash.familyconnect.ui.chat.CallRecordWording

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StatisticsScreen(
    onBack: () -> Unit,
    viewModel: StatisticsViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.s_statistics)) },
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
        when (val current = state) {
            is StatisticsViewModel.State.Loading ->
                Column(
                    modifier = Modifier.fillMaxSize().padding(padding),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) { CircularProgressIndicator() }

            is StatisticsViewModel.State.Failed ->
                Column(
                    modifier = Modifier.fillMaxSize().padding(padding).padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        stringResource(R.string.s_couldnt_load_statistics),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Spacer(Modifier.padding(4.dp))
                    Text(
                        stringResource(R.string.s_check_connection_try_again),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.padding(8.dp))
                    // The copy says "try again"; this is the way to.
                    Button(onClick = viewModel::load) {
                        Text(stringResource(R.string.s_retry))
                    }
                }

            is StatisticsViewModel.State.Loaded -> {
                val stats = current.stats
                LazyColumn(
                    modifier = Modifier.fillMaxSize().readableColumn(),
                    contentPadding = PaddingValues(
                        top = padding.calculateTopPadding() + 8.dp,
                        bottom = padding.calculateBottomPadding() + 24.dp,
                    ),
                ) {
                    item { SectionHeader(stringResource(R.string.s_the_family)) }
                    item { StatRow(stringResource(R.string.s_members), "${stats.totals.members}") }
                    item { StatRow(stringResource(R.string.s_messages), "${stats.totals.messages}") }
                    item {
                        StatRow(stringResource(R.string.s_board_notes), "${stats.totals.boardNotes}")
                    }

                    item { SectionHeader(stringResource(R.string.s_attachments)) }
                    item { StatRow(stringResource(R.string.s_photos), "${stats.totals.attachments.photo}") }
                    item { StatRow(stringResource(R.string.s_videos), "${stats.totals.attachments.video}") }
                    item { StatRow(stringResource(R.string.s_audio), "${stats.totals.attachments.audio}") }
                    item { StatRow(stringResource(R.string.s_files), "${stats.totals.attachments.file}") }
                    item {
                        StatRow(
                            stringResource(R.string.s_sent),
                            formatBytes(context, stats.totals.attachments.bytes),
                        )
                    }
                    stats.totals.attachments.storedBytes?.let { stored ->
                        item {
                            StatRow(
                                stringResource(R.string.s_on_disk),
                                formatBytes(context, stored),
                            )
                        }
                        val saved = stats.totals.attachments.bytes - stored
                        if (saved > 0) {
                            item {
                                // The gap is the point: identical bytes are
                                // kept once per family.
                                Text(
                                    text = stringResource(
                                        R.string.s_saved_by_one_copy,
                                        formatBytes(context, saved),
                                    ),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                                )
                            }
                        }
                    }

                    val assistantRows = StatisticsLines.assistantRows(stats.totals.ai, context)
                    if (assistantRows.isNotEmpty()) {
                        item { SectionHeader(stringResource(R.string.s_assistant)) }
                        items(assistantRows) { (label, value) -> StatRow(label, value) }
                    }

                    item { SectionHeader(stringResource(R.string.s_who_sends_what)) }
                    items(stats.members, key = { it.userId }) { member ->
                        MemberRow(member = member, context = context)
                    }
                }
            }
        }
    }
}

@Composable
private fun SectionHeader(title: String) {
    Column {
        Spacer(Modifier.padding(6.dp))
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Text(
            text = title,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
    }
}

@Composable
private fun StatRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge)
        Spacer(Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun MemberRow(member: MemberStatsDto, context: Context) {
    Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(member.displayName, style = MaterialTheme.typography.bodyLarge)
            Spacer(Modifier.weight(1f))
            Text("${member.messages}", style = MaterialTheme.typography.bodyLarge)
        }
        Text(
            text = StatisticsLines.summaryFor(member, context),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * The words of the page that are decisions, apart from Compose so a test
 * can hold them: the same rows and the same member line iOS, the web and
 * Windows draw.
 */
internal object StatisticsLines {

    /**
     * The assistant section's rows, label to value — empty when the family
     * has used none of it, and then no section is drawn. A family that only
     * asked for pictures or for transcripts still gets one: transcripts are
     * not counted as questions, and a section that hid them would make
     * what they cost read as free.
     */
    fun assistantRows(ai: AiStatsDto, context: Context): List<Pair<String, String>> {
        if (ai.questions <= 0 && ai.images <= 0 && ai.transcripts <= 0) return emptyList()
        return buildList {
            add(context.getString(R.string.s_questions) to "${ai.questions}")
            add(context.getString(R.string.s_tokens) to "${ai.promptTokens + ai.completionTokens}")
            if (ai.images > 0) add(context.getString(R.string.s_stats_pictures) to "${ai.images}")
            // Billed by length, not tokens — so the length is shown.
            if (ai.transcripts > 0) {
                add(context.getString(R.string.s_stats_recordings_as_text) to "${ai.transcripts}")
                add(context.getString(R.string.s_stats_recording_time) to recordingTime(ai.transcriptDurationMs))
            }
        }
    }

    /** "3:42", or "1:03:42" past an hour — rounded to the nearest second. */
    fun recordingTime(durationMs: Long): String =
        CallRecordWording.duration(((durationMs.coerceAtLeast(0) + 500) / 1000).coerceAtMost(Int.MAX_VALUE.toLong()).toInt())

    /** One line under a member: what they sent besides words. */
    fun summaryFor(member: MemberStatsDto, context: Context): String {
        val parts = buildList {
            if (member.attachments.count > 0) {
                add(
                    context.getString(
                        R.string.s_attachments_and_size,
                        member.attachments.count,
                        formatBytes(context, member.attachments.bytes),
                    ),
                )
            }
            if (member.ai.questions > 0) {
                add(context.getString(R.string.s_questions_to_assistant, member.ai.questions))
            }
            if (member.ai.images > 0) {
                add(context.getString(R.string.s_pictures_from_assistant, member.ai.images))
            }
            if (member.ai.transcripts > 0) {
                add(context.getString(R.string.s_recordings_as_text, member.ai.transcripts))
            }
        }
        return if (parts.isEmpty()) context.getString(R.string.s_words_only) else parts.joinToString(" · ")
    }
}

/** `1.2 MB`, in the reader's own units and language. */
private fun formatBytes(context: Context, bytes: Long): String =
    Formatter.formatFileSize(context, bytes)
