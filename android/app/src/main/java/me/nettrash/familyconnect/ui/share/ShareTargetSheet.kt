/*
 * ShareTargetSheet.kt
 * Family Connect (Android)
 *
 * The chat picker an OS share lands on: family chat first (ChatDao's own
 * order), direct chats by recency — and NEVER the assistant's private
 * thread. Picking a chat navigates there with the prepared items waiting
 * in ShareStash; dismissing discards them. Nothing here auto-sends.
 */

package me.nettrash.familyconnect.ui.share

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.ChatEntity
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.ui.components.Avatar
import me.nettrash.familyconnect.ui.components.EmptyState
import javax.inject.Inject

/**
 * The chats a share may land in: family or direct, NEVER `kind == "ai"` —
 * the assistant's private thread takes no shared files, and offering it
 * would offer a send the server refuses. Order is the DAO's (family chat
 * pinned first, then recency), so this only filters. Pure, and pinned by
 * ShareTargetsTest.
 */
fun shareTargets(chats: List<ChatEntity>): List<ChatEntity> = chats.filter { it.kind != "ai" }

@HiltViewModel
class ShareTargetViewModel @Inject constructor(
    chatDao: ChatDao,
    familyRepository: FamilyRepository,
) : ViewModel() {
    /** Null while Room has not answered — the sheet shows nothing yet, not "no chats". */
    val chats: StateFlow<List<ChatEntity>?> = chatDao.observeChats()
        .map(::shareTargets)
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * userId → profile-picture version — the same derivation
     * ChatListViewModel makes, so a direct chat shows the same face here
     * that its row shows in the chat list.
     */
    val avatarVersions: StateFlow<Map<Long, Long>> = familyRepository.observeMembers()
        .map { members -> members.associate { it.userId to it.avatarVersion } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyMap())
}

/** The copy is still running; a sheet that says so beats a share that looks dropped. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SharePreparingSheet(onDismiss: () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            Spacer(Modifier.size(16.dp))
            Text(
                text = stringResource(R.string.s_share_choose_chat),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Spacer(Modifier.height(24.dp))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShareTargetSheet(
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
    viewModel: ShareTargetViewModel = hiltViewModel(),
) {
    val chats by viewModel.chats.collectAsStateWithLifecycle()
    val avatarVersions by viewModel.avatarVersions.collectAsStateWithLifecycle()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Text(
            text = stringResource(R.string.s_share_choose_chat),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 16.dp),
        )
        val list = chats
        when {
            list == null -> Unit // Room has not answered; flashing "no chats" would lie.
            list.isEmpty() -> Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 24.dp),
                contentAlignment = Alignment.Center,
            ) {
                EmptyState(
                    icon = Icons.AutoMirrored.Filled.Chat,
                    title = stringResource(R.string.s_no_chats_yet),
                    subtitle = stringResource(
                        R.string.s_the_family_chat_appears_as_soon_as_you_re_connected,
                    ),
                )
            }
            else -> Column {
                list.forEach { chat ->
                    ListItem(
                        headlineContent = { Text(chat.title) },
                        leadingContent = {
                            // The same marks the chat list draws: the family
                            // chat is everyone's, so no one person's color.
                            if (chat.kind == "family") {
                                Box(
                                    modifier = Modifier
                                        .size(48.dp)
                                        .background(
                                            MaterialTheme.colorScheme.primaryContainer,
                                            CircleShape,
                                        ),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Icon(
                                        imageVector = Icons.Outlined.Home,
                                        contentDescription = null,
                                        modifier = Modifier.size(22.dp),
                                        tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                    )
                                }
                            } else {
                                Avatar(
                                    name = chat.title,
                                    userId = chat.peerUserId ?: chat.id,
                                    size = 48,
                                    avatarVersion = chat.peerUserId
                                        ?.let { avatarVersions[it] } ?: 0L,
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        modifier = Modifier.clickable { onPick(chat.id) },
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}
