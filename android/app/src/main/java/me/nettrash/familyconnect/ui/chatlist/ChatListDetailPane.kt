/*
 * ChatListDetailPane.kt
 * Family Connect (Android)
 *
 * The tablet's shape of the chat list: the list as a pane on the left,
 * the open thread on the right, and — with nothing selected — the
 * family's name, whole, where a tablet has the room for it. The phone's
 * shape (list, then a pushed thread) is untouched: AppNavHost chooses
 * between the two by the window (isTwoPaneWindow), the way the iPad
 * client chooses a NavigationSplitView over a NavigationStack.
 *
 * The thread pane is a NavHost of its own, one entry deep, keyed on the
 * selected chat: ChatScreen reads its chat id from the back-stack entry
 * (ChatViewModel's SavedStateHandle), so giving it an entry is what lets
 * it run unchanged here. Its Back — the arrow, the gesture, the pop when
 * a chat vanishes — clears the selection rather than leaving the pane.
 *
 * Push taps, the share picker and a call-back still push Routes.CHAT on
 * the outer NavHost, a full-screen thread over this pane, and Back there
 * returns here — the phone's route, unchanged, on a wider screen.
 *
 * iOS counterpart: ChatListView.splitShape.
 */

package me.nettrash.familyconnect.ui.chatlist

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.navigation.Routes
import me.nettrash.familyconnect.ui.chat.ChatScreen
import me.nettrash.familyconnect.ui.group.GroupInfoScreen
import me.nettrash.familyconnect.ui.components.LIST_PANE_WIDTH_DP
import me.nettrash.familyconnect.ui.thread.ThreadScreen
import me.nettrash.familyconnect.ui.polls.OpenPollsScreen

/** The list pane's width — shared with isTwoPane, which keeps the conversation beside it at least 400dp. */
private val LIST_PANE_WIDTH = LIST_PANE_WIDTH_DP.dp

@Composable
fun ChatListDetailPane(
    onOpenSettings: () -> Unit,
    onOpenBoard: () -> Unit,
    onNewFamily: () -> Unit = {},
    onJoinFamily: () -> Unit = {},
    viewModel: ChatListViewModel = hiltViewModel(),
) {
    val chats by viewModel.chats.collectAsStateWithLifecycle()
    val members by viewModel.pickableMembers.collectAsStateWithLifecycle()
    val families by viewModel.families.collectAsStateWithLifecycle()
    // Survives rotation and a fold; a process death lands on the empty
    // pane, which is where a launch lands too. Keyed by the family shown:
    // a switch is a different list, and the thread beside it goes.
    var selectedChatId by rememberSaveable(families.currentFamilyId) { mutableStateOf<Long?>(null) }
    // The CURRENT family's name (docs/protocol.md, "Several families") —
    // or, before the family list is known, the family chat's title, which
    // the server sets to it. `chats` is the current family's already, so
    // its family chat is this family's. The roster's pickable members plus
    // me is everyone still in.
    val familyName = families.currentName ?: chats?.firstOrNull { it.kind == "family" }?.title
    val memberCount = members.size + 1

    Row(modifier = Modifier.fillMaxSize()) {
        Box(
            modifier = Modifier
                .width(LIST_PANE_WIDTH)
                .fillMaxHeight(),
        ) {
            ChatListScreen(
                onOpenChat = { selectedChatId = it },
                onOpenSettings = onOpenSettings,
                onOpenBoard = onOpenBoard,
                selectedChatId = selectedChatId,
                onNewFamily = onNewFamily,
                onJoinFamily = onJoinFamily,
                viewModel = viewModel,
            )
        }
        VerticalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Box(
            modifier = Modifier
                .weight(1f)
                .fillMaxHeight(),
        ) {
            val chatId = selectedChatId
            if (chatId == null) {
                FamilyEmptyPane(familyName = familyName, memberCount = memberCount)
            } else {
                // A new NavHost per chat: the thread's state — its scroll,
                // its draft, its unread anchor — belongs to one chat, and a
                // host reused across chats would keep the old one's.
                key(chatId) {
                    val detailNav = rememberNavController()
                    NavHost(
                        navController = detailNav,
                        startDestination = Routes.chat(chatId),
                    ) {
                        composable(
                            route = Routes.CHAT,
                            arguments = listOf(navArgument("chatId") { type = NavType.LongType }),
                        ) {
                            ChatScreen(
                                onBack = { selectedChatId = null },
                                onOpenPolls = { id -> detailNav.navigate(Routes.openPolls(id)) },
                                onOpenThread = { id, rootId -> detailNav.navigate(Routes.thread(id, rootId)) },
                                onOpenChat = { id -> selectedChatId = id },
                                onOpenGroupInfo = { id -> detailNav.navigate(Routes.groupInfo(id)) },
                            )
                        }
                        composable(
                            Routes.GROUP_INFO,
                            arguments = listOf(navArgument("chatId") { type = NavType.LongType }),
                        ) {
                            GroupInfoScreen(
                                onBack = { detailNav.popBackStack() },
                                // The group is gone, and the thread with it.
                                onGroupGone = { selectedChatId = null },
                            )
                        }
                        // The chat's own surfaces open inside the detail
                        // pane, as they do on a phone — a chip or a menu
                        // row that does nothing here is worse than none.
                        composable(
                            Routes.OPEN_POLLS,
                            arguments = listOf(navArgument("chatId") { type = NavType.LongType }),
                        ) { entry ->
                            OpenPollsScreen(
                                chatId = entry.arguments?.getLong("chatId") ?: 0L,
                                onBack = { detailNav.popBackStack() },
                            )
                        }
                        composable(
                            Routes.THREAD,
                            arguments = listOf(
                                navArgument("chatId") { type = NavType.LongType },
                                navArgument("rootId") { type = NavType.LongType },
                            ),
                        ) { entry ->
                            ThreadScreen(
                                chatId = entry.arguments?.getLong("chatId") ?: 0L,
                                rootId = entry.arguments?.getLong("rootId") ?: 0L,
                                onBack = { detailNav.popBackStack() },
                                // The chat is gone, and the thread with it.
                                onChatGone = { selectedChatId = null },
                                onOpenChat = { id -> selectedChatId = id },
                            )
                        }
                    }
                }
            }
        }
    }
}

/**
 * Nothing selected: the family's name, whole — the one place on a
 * tablet with the room for a long one — over an invitation to pick a
 * chat. The iPad's empty detail column, in Compose.
 */
@Composable
private fun FamilyEmptyPane(familyName: String?, memberCount: Int) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier
                .widthIn(max = 420.dp)
                .padding(40.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            FamilyAvatarMark(size = 96)
            Spacer(Modifier.height(16.dp))
            Text(
                text = familyName ?: stringResource(R.string.s_family),
                style = MaterialTheme.typography.headlineSmall,
                textAlign = TextAlign.Center,
                maxLines = 3,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = pluralStringResource(R.plurals.p_members, memberCount, memberCount),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(14.dp))
            Text(
                text = stringResource(R.string.s_pick_a_chat),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
