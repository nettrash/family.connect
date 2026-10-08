/*
 * StickerViewModel.kt
 * Family Connect (Android)
 *
 * Everything the chat's STICKERS need from a screen, in one place
 * (docs/protocol.md, "Sticker pack"): the panel in the composer, the larger
 * view a tap opens, and the pack's management on the Family screen.
 *
 * Its own ViewModel rather than more of ChatViewModel, ThreadViewModel and
 * FamilyAdminViewModel, because all three screens need the same few things
 * and none of them owns the pack. Each screen asks Hilt for one; the state
 * underneath is the app-scoped [PackRepository], so they agree.
 *
 * "Sticker" here is the chat picture. The board's notes — which this
 * codebase has always called stickers internally — are no part of this.
 *
 * iOS counterpart: the sticker panel's model in the chat view.
 */

package me.nettrash.familyconnect.ui.stickers

import android.content.Context
import android.net.Uri
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.PackItemEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.data.repo.AvatarSource
import me.nettrash.familyconnect.data.repo.PackLimits
import me.nettrash.familyconnect.data.repo.PackPicture
import me.nettrash.familyconnect.data.repo.PackRepository
import me.nettrash.familyconnect.data.repo.PackRules
import me.nettrash.familyconnect.data.repo.StickerSender
import me.nettrash.familyconnect.data.repo.packLimits
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import java.io.File
import javax.inject.Inject

@HiltViewModel
class StickerViewModel @Inject constructor(
    /** For `getString` only — see ChatViewModel for why that is acceptable. */
    @param:ApplicationContext private val appContext: Context,
    private val pack: PackRepository,
    private val sender: StickerSender,
    private val attachments: AttachmentRepository,
    /**
     * The bounded reader the profile-picture picker uses: a picked Uri's
     * bytes, or null when they cannot be read or are absurdly large.
     */
    private val source: AvatarSource,
    private val settings: SettingsRepository,
    @param:AppScope private val appScope: CoroutineScope,
) : ViewModel() {

    /**
     * The pack's ceilings — and, by being null, the news that this server
     * has no packs. Every sticker affordance is drawn behind it: no button
     * in the composer, no section on the Family screen, no "Add to family
     * stickers" — rather than a 404 when somebody taps one.
     */
    val limits: StateFlow<PackLimits?> = settings.state.map { it.packLimits }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /** The pack, in the order it was added. */
    val items: StateFlow<List<PackItemEntity>> = pack.observeItems()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    /** What this DEVICE sent most recently, newest first, and still in the pack. */
    val recents: StateFlow<List<PackItemEntity>> =
        combine(pack.observeItems(), settings.state.map { it.packRecents }) { held, recentIds ->
            PackRules.recents(recentIds, held) { it.id }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    private val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    private val isOwner: StateFlow<Boolean> =
        settings.state.map { it.familyStatus == FamilyStatus.OWNER }
            .stateIn(viewModelScope, SharingStarted.Eagerly, false)

    /** An add is running: the picker's button waits rather than stacking a second. */
    private val _adding = MutableStateFlow(false)
    val adding: StateFlow<Boolean> = _adding

    /**
     * One-shot, already-localised sentences for the screen to show. No
     * replay: each reports something that just happened, and a replayed one
     * would fire again after a rotation.
     */
    private val _notices = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val notices: SharedFlow<String> = _notices

    /** Whether the remove action is drawn — [PackRules.canRemove] has the rule. */
    fun canRemove(item: PackItemEntity): Boolean =
        PackRules.canRemove(item.addedBy, myUserId.value, isOwner.value)

    /** A pack item's original bytes, for a cell to draw. */
    suspend fun packFile(item: PackItemEntity): File? =
        item.attachment?.let { pack.fileFor(it) }

    /**
     * One tap sends. App scope, not the ViewModel's: the staging must not
     * be cancelled by the panel closing, which the same tap causes.
     */
    fun send(item: PackItemEntity, chatId: Long, replyTo: ReplyToDto?) {
        appScope.launch {
            when (sender.send(item, chatId, replyTo)) {
                // The bubble is the report.
                StickerSender.Result.QUEUED -> Unit
                StickerSender.Result.NO_BYTES ->
                    _notices.tryEmit(appContext.getString(R.string.s_sticker_not_available_offline))
                StickerSender.Result.TOO_LARGE -> report(PackRepository.AddResult.TOO_LARGE)
            }
        }
    }

    /**
     * Whether "Add to family stickers" is offered on a sticker somebody
     * sent: only on a server with packs, and only when the pack does not
     * already hold those bytes (docs/protocol.md, "Sticker pack").
     *
     * A wrong "yes" is harmless — the server answers such an add `200` with
     * the item that was there — so a picture this device cannot read yet is
     * offered rather than hidden.
     */
    suspend fun offersAdd(attachment: AttachmentDto): Boolean {
        if (limits.value == null || attachment.id < 0) return false
        if (!PackPicture.isStickerType(attachment.mime)) return false
        return pack.holding(attachment.mime, attachment.size) {
            attachments.originalFile(attachment.id)?.let { file ->
                withContext(Dispatchers.IO) { runCatching { file.readBytes() }.getOrNull() }
            }
        } == null
    }

    /** "Add to family stickers": the message's own bytes, uploaded again unprepared. */
    fun addFromMessage(attachment: AttachmentDto, onDone: (Boolean) -> Unit = {}) {
        if (_adding.value) return
        _adding.value = true
        appScope.launch {
            val result = attachments.originalFile(attachment.id)
                ?.let { file -> pack.addFromMessage(attachment, file) }
                ?: PackRepository.AddResult.FAILED
            report(result)
            _adding.value = false
            onDone(result == PackRepository.AddResult.ADDED || result == PackRepository.AddResult.ALREADY_IN_PACK)
        }
    }

    /**
     * Add a picked picture to the pack.
     *
     * A `.webp` or `.png` goes up exactly as it is; any other STILL
     * picture, or a still one over the ceiling, is fitted whole into
     * 512 x 512 with its transparency kept ([PackPicture]); an animated one
     * that is not a WebP is refused in words. NEVER through MediaPrep: that
     * is the photo path, and it writes JPEG.
     */
    fun addPicked(uri: Uri, label: String?) {
        if (_adding.value) return
        val ceiling = limits.value ?: return
        _adding.value = true
        appScope.launch {
            val bytes = source.read(uri)
            val outcome = if (bytes == null) {
                PackPicture.Outcome.Unreadable
            } else {
                withContext(Dispatchers.Default) { PackPicture.prepare(bytes, ceiling.maxItemBytes) }
            }
            when (outcome) {
                is PackPicture.Outcome.Ready -> report(pack.add(outcome.picture, label))
                PackPicture.Outcome.TooLarge -> report(PackRepository.AddResult.TOO_LARGE)
                // Refused in words, never flattened to its first frame.
                PackPicture.Outcome.AnimatedNotWebp ->
                    _notices.tryEmit(appContext.getString(R.string.s_sticker_animated_must_be_webp))
                PackPicture.Outcome.Unreadable ->
                    _notices.tryEmit(appContext.getString(R.string.s_sticker_unreadable))
            }
            _adding.value = false
        }
    }

    fun remove(item: PackItemEntity) {
        appScope.launch {
            when (pack.remove(item.id)) {
                PackRepository.RemoveResult.REMOVED -> Unit
                PackRepository.RemoveResult.NOT_ALLOWED ->
                    _notices.tryEmit(appContext.getString(R.string.s_sticker_remove_not_allowed))
                PackRepository.RemoveResult.FAILED ->
                    _notices.tryEmit(appContext.getString(R.string.s_sticker_remove_failed))
            }
        }
    }

    /** Every outcome of an add is SAID — a sticker that silently did not appear is one its adder goes looking for. */
    private suspend fun report(result: PackRepository.AddResult) {
        val ceiling = settings.state.first().packLimits
        val message = when (result) {
            PackRepository.AddResult.ADDED -> appContext.getString(R.string.s_sticker_added)
            PackRepository.AddResult.ALREADY_IN_PACK ->
                appContext.getString(R.string.s_sticker_already_in_pack)
            PackRepository.AddResult.FULL ->
                appContext.getString(R.string.s_sticker_pack_full, ceiling?.maxItems ?: 0)
            PackRepository.AddResult.TOO_LARGE -> appContext.getString(
                R.string.s_sticker_too_large,
                android.text.format.Formatter.formatShortFileSize(appContext, ceiling?.maxItemBytes ?: 0L),
            )
            PackRepository.AddResult.LABEL_TOO_LONG -> appContext.getString(
                R.string.s_sticker_label_too_long,
                PackPicture.MAX_LABEL_CHARS,
            )
            PackRepository.AddResult.FAILED -> appContext.getString(R.string.s_sticker_add_failed)
        }
        _notices.tryEmit(message)
    }
}
