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
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.navigation.Routes
import me.nettrash.familyconnect.ui.chat.ChatScreen

/** The list pane's width: the iPad sidebar's 320pt, which fits a two-line family name beside its time. */
private val LIST_PANE_WIDTH = 340.dp

@Composable
fun ChatListDetailPane(
    onOpenSettings: () -> Unit,
    onOpenBoard: () -> Unit,
    viewModel: ChatListViewModel = hiltViewModel(),
) {
    // Survives rotation and a fold; a process death lands on the empty
    // pane, which is where a launch lands too.
    var selectedChatId by rememberSaveable { mutableStateOf<Long?>(null) }
    val chats by viewModel.chats.collectAsStateWithLifecycle()
    val members by viewModel.pickableMembers.collectAsStateWithLifecycle()
    // The family chat's title IS the family's name (the server titles it
    // so); the roster's pickable members plus me is everyone still in.
    val familyName = chats?.firstOrNull { it.kind == "family" }?.title
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
                            ChatScreen(onBack = { selectedChatId = null })
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
