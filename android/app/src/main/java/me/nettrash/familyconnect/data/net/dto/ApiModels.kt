/*
 * ApiModels.kt
 * Family Connect (Android)
 *
 * REST wire shapes, transcribed 1:1 from docs/protocol.md ("Objects" +
 * per-endpoint tables). Every field is @SerialName'd to the protocol's
 * snake_case so Kotlin property names stay idiomatic without a global
 * naming strategy. Optional fields default to null — combined with
 * `ignoreUnknownKeys` this is what makes v1 clients forward-compatible
 * with fields the server adds later.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ApiModels.swift
 */

package me.nettrash.familyconnect.data.net.dto

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive

// -- Objects ---------------------------------------------------------------

/**
 * A day and a month, with **no year** (docs/protocol.md, "Birthdays").
 *
 * Nobody should have to publish their age to be wished a happy birthday,
 * and the year is the only part of a date that carries one. Two
 * consequences ride on that and both are deliberate: 29 February is a
 * perfectly good birthday, because there is no year for it to fail to
 * exist in, and nothing here can compute an age — so nothing should try.
 *
 * One object rather than two sibling fields, because the two halves are a
 * single fact: "month but no day" is not a state anything has to handle.
 */
@Serializable
data class BirthdayDto(
    val month: Int,
    val day: Int,
)

@Serializable
data class UserDto(
    val id: Long,
    val username: String,
    @SerialName("display_name") val displayName: String,
    // Absent on the embedded user of `member_joined` frames.
    @SerialName("created_at") val createdAt: String? = null,
    /**
     * Bumps on every profile-picture change; 0 = no picture. The default
     * is what lets a client talk to a server older than the avatars
     * release — and it doubles as the cache key, so a changed picture can
     * never be served from a stale entry.
     */
    @SerialName("avatar_version") val avatarVersion: Long = 0,
    /** Present when (and only when) one is set; absent means unset. */
    val birthday: BirthdayDto? = null,
    /**
     * True when (and only when) this account has been DELETED
     * (docs/protocol.md, "Deleting an account"). Absent means false —
     * the flag is never sent as `false`, so the nullable default is the
     * whole of the compatibility story with a server that predates it.
     *
     * Such an account has no usable username, no picture (avatar_version
     * is 0) and no birthday, and its `display_name` is the server's
     * ENGLISH placeholder. A client that understands the flag draws its
     * own translation instead — see [isDeleted] and its call sites.
     */
    val deleted: Boolean? = null,
) {
    val isDeleted: Boolean get() = deleted == true
}

@Serializable
data class MemberDto(
    val id: Long,
    val username: String,
    @SerialName("display_name") val displayName: String,
    /**
     * "owner" | "member" — and NULL on a tombstone, which is the whole
     * reason this is optional: a deleted account holds no role
     * (docs/protocol.md, "Deleting an account"), so `former_members` and
     * the `member_deleted` frame both omit the key. Live members always
     * carry it.
     */
    val role: String? = null,
    /** See UserDto.avatarVersion. */
    @SerialName("avatar_version") val avatarVersion: Long = 0,
    /** See UserDto.birthday — the same field, on the roster shape. */
    val birthday: BirthdayDto? = null,
    /**
     * See UserDto.deleted. A deleted account is NEVER in `members`; it
     * appears in `former_members` (see [FamilyMineResponse]) and in the
     * `member_deleted` frame, and it exists there for one reason — so a
     * client can still put a name to the messages, notes and reactions it
     * left behind.
     */
    val deleted: Boolean? = null,
) {
    val isDeleted: Boolean get() = deleted == true
}

@Serializable
data class FamilyDto(
    val id: Long,
    val name: String,
    @SerialName("join_policy") val joinPolicy: String,
    @SerialName("created_at") val createdAt: String? = null,
    // Present when (and only when) the caller is the owner.
    @SerialName("invite_code") val inviteCode: String? = null,
    /**
     * The one language the family speaks, or null for UNSET — and unset
     * is NOT English (docs/protocol.md, "The family's language"). The key
     * is absent until an owner chooses one, so the default is what lets
     * this decode against a server that predates the field; it must stay
     * distinguishable from `"en"` everywhere, on the wire and on screen.
     */
    val language: String? = null,
    /**
     * Whether a mention of the assistant in the family chat is shown the
     * recent history of that chat, or only the message that mentioned it.
     *
     * ALWAYS present on the wire, unlike the two fields above: it is a
     * boolean with a real default and no "unset" for a missing key to
     * mean. The default here is therefore purely the compatibility rule —
     * an older server sends nothing, and `true` is what such a server's
     * successor does.
     */
    @SerialName("ai_history") val aiHistory: Boolean = true,
    /**
     * The most members this family admits, or null for NO cap of the
     * owner's own — in which case the operator's ceiling
     * (`MeResponse.maxFamilyMembers`) is what binds at the join door.
     *
     * Absent from the wire until an owner sets one, so null is both "no
     * cap" and "a server that predates the field", and those mean the
     * same thing here (docs/protocol.md, `PATCH /families/mine`).
     */
    @SerialName("max_members") val maxMembers: Int? = null,
)

@Serializable
data class PendingJoinRequestDto(
    @SerialName("family_id") val familyId: Long,
    @SerialName("family_name") val familyName: String,
    @SerialName("created_at") val createdAt: String,
)

@Serializable
data class JoinRequestDto(
    val id: Long,
    val user: UserDto,
    @SerialName("created_at") val createdAt: String,
)

@Serializable
data class ChatDto(
    val id: Long,
    val kind: String,
    val title: String,
    @SerialName("peer_user_id") val peerUserId: Long? = null,
)

@Serializable
data class ReactionDto(
    @SerialName("user_id") val userId: Long,
    val emoji: String,
)

/**
 * One option of a poll, with the FULL current list of user ids that
 * chose it (docs/protocol.md, "Polls").
 *
 * Votes are attributed on purpose: one frame is serialised once and sent
 * to every connection, so a field whose value depends on who is reading
 * it — "did I vote" — cannot exist. Clients derive that from the list,
 * which is also what lets a bubble draw who has NOT voted yet.
 *
 * [id] is the server's, and it is the only thing `PUT .../vote` names.
 * Options are fixed at creation and never gain, lose or rename one.
 */
@Serializable
data class PollOptionDto(
    val id: Long,
    val text: String,
    /**
     * Everyone who currently holds this option, in the order they voted.
     *
     * No default, like every field of [PollDto]: an empty list is a real
     * answer ("nobody chose this") and is ALWAYS on the wire, and a
     * default would be dropped on re-encode by encodeDefaults=false —
     * turning a stored or forwarded poll into one whose options had no
     * votes key at all.
     */
    val votes: List<Long>,
)

/**
 * What makes a message votable (docs/protocol.md, "Polls").
 *
 * The QUESTION is deliberately not here: it is the message BODY, which
 * is what lets a chat-list preview, a push alert and a reply excerpt all
 * read a poll without one new case between them — and what lets a client
 * that has never heard of polls draw it as an ordinary message and lose
 * only the buttons.
 *
 * [pollSeq] and [closed] carry NO default, unlike every other field
 * added to this file since v1: the protocol says both are ALWAYS present
 * ("a poll has a sequence from the moment it exists, and `closed` is a
 * boolean with a real default, so there is no unset for a missing key to
 * mean"), and the server serialises them unconditionally. Defaults here
 * would buy nothing and would cost the round-trip: `encodeDefaults=false`
 * drops a defaulted value, so a stored or re-encoded poll would come out
 * missing the two fields the protocol guarantees.
 *
 * There is no multiple choice. A poll takes exactly one answer per
 * member, changeable — see [optionHeldBy].
 */
@Serializable
data class PollDto(
    @SerialName("poll_seq") val pollSeq: Long,
    val closed: Boolean,
    val options: List<PollOptionDto>,
) {
    /** The option this user currently holds, or null when they have not voted. */
    fun optionHeldBy(userId: Long): PollOptionDto? =
        options.firstOrNull { option -> option.votes.any { it == userId } }

    /** Everybody who has voted, each counted once (a vote is one option). */
    val voters: Set<Long> get() = options.flatMapTo(LinkedHashSet()) { it.votes }

    /** Votes cast in total — the denominator of every option's share. */
    val totalVotes: Int get() = options.sumOf { it.votes.size }
}

/**
 * The quoted message on a reply — as much of it as a bubble needs to draw
 * the quote without holding the original. The server recomputes this on
 * every read, so it follows the quoted message rather than freezing at send
 * time (docs/protocol.md, "Replies").
 */
@Serializable
data class ReplyToDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("sender_id") val senderId: Long,
    val excerpt: String,
    /**
     * What the quoted message was itself answering — one level, no more.
     * Null is normal: the quoted message was not a reply, or its own parent
     * has been swept by retention (protocol.md, "Replies").
     */
    val parent: QuotedParentDto? = null,
) {
    companion object {
        /**
         * Longest excerpt, in Unicode CODE POINTS — matching
         * `ReplyTo::MAX_EXCERPT_CHARS` on the server and
         * `ReplyToSnapshot.maxExcerptScalars` on iOS.
         */
        const val MAX_EXCERPT_CHARS = 120

        /**
         * Cut like the server does. `String.take` counts UTF-16 CODE
         * UNITS, so it both keeps the wrong amount of text for
         * astral characters and can slice a surrogate pair in half —
         * which renders as a replacement glyph, in a quote the user did
         * not write. Code points, therefore, like `chars().take(120)`
         * server-side.
         */
        fun excerpt(body: String): String {
            if (body.codePointCount(0, body.length) <= MAX_EXCERPT_CHARS) return body
            val end = body.offsetByCodePoints(0, MAX_EXCERPT_CHARS)
            return body.substring(0, end)
        }
    }
}

/**
 * The second and LAST level of a quote (protocol.md, "Replies").
 *
 * Deliberately not the same type as [ReplyToDto]: with no `parent` of its
 * own there is nothing for a third level to decode into, so the cap is
 * structural rather than a rule to remember.
 */
@Serializable
data class QuotedParentDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("sender_id") val senderId: Long,
    val excerpt: String,
)

@Serializable
data class MessageDto(
    val id: Long,
    @SerialName("chat_id") val chatId: Long,
    @SerialName("sender_id") val senderId: Long,
    @SerialName("client_msg_id") val clientMsgId: String,
    val body: String,
    @SerialName("created_at") val createdAt: String,
    // Both absent when (and only when) the message was never reacted
    // to; after clearing, reactions is [] with the seq still present.
    val reactions: List<ReactionDto>? = null,
    @SerialName("reaction_seq") val reactionSeq: Long? = null,
    // Present when (and only when) this message is a reply.
    @SerialName("reply_to") val replyTo: ReplyToDto? = null,
    // Both present when (and only when) the body has been edited. The seq
    // is the apply guard — see MessageRepository.applyBody.
    @SerialName("edited_at") val editedAt: String? = null,
    @SerialName("edit_seq") val editSeq: Long? = null,
    // The FIRST element of `attachments`, kept for clients that predate
    // plurality (docs/protocol.md, "Objects"). A client that reads
    // `attachments` ignores it; the two are never present without each
    // other. This client reads through [resolvedAttachments].
    val attachment: AttachmentDto? = null,
    /**
     * Present when (and only when) the message carries photos, videos,
     * audio, files or a location — 1 to 10 of them, in the order the
     * sender chose; absent (never an empty array) on a message that
     * carries none (docs/protocol.md, "Objects").
     */
    val attachments: List<AttachmentDto>? = null,
    /**
     * Present when (and only when) this message is a poll — and polls
     * exist in the FAMILY CHAT only (docs/protocol.md, "Polls"). The
     * question is [body]; this carries the options and the votes.
     *
     * An ABSENT poll never means "the poll went away": a poll dies only
     * with its message. Every apply path guards on that — see
     * MessageRepository.applyEmbeddedPoll.
     */
    val poll: PollDto? = null,
    /**
     * Present when (and only when) this message is the RECORD of a voice
     * call (docs/protocol.md, "Voice calls"). The body is then the
     * server's English placeholder — "Voice call" / "Missed voice call" —
     * which a client that knows this object never shows: the bubble draws
     * its own wording from the outcome, the duration and which side of
     * the call it was on.
     */
    val call: CallDto? = null,
) {
    /**
     * The read rule, in one place: prefer `attachments`, fall back to the
     * legacy `attachment` (its first element, on a server that sends
     * both). Empty for a message that carries none — never null, because
     * every consumer walks the list.
     */
    val resolvedAttachments: List<AttachmentDto>
        get() = attachments ?: attachment?.let(::listOf).orEmpty()
}

/**
 * The record of a voice call, on the message that is its history entry
 * (docs/protocol.md, `Call`).
 *
 * [durationSecs] is present when (and only when) the call was ever
 * answered — the seconds from the answer to the end on the server's
 * clock — so a "failed" call may carry one and a "missed" call never does.
 */
@Serializable
data class CallDto(
    /** "completed" | "missed" | "declined" | "failed". */
    val outcome: String,
    @SerialName("duration_secs") val durationSecs: Int? = null,
    /**
     * True when it was a VIDEO call (docs/protocol.md, "Video") — present
     * on the wire when and only when true, absent on a voice call like
     * every optional field. The kind was fixed when the call was placed;
     * the record simply reports it.
     */
    val video: Boolean = false,
) {
    companion object {
        const val COMPLETED = "completed"
        const val MISSED = "missed"
        const val DECLINED = "declined"
        const val FAILED = "failed"
    }
}

/**
 * One ICE candidate, as the signalling frames carry it (docs/protocol.md,
 * "Voice calls"). `sdp_mid` and `sdp_mline_index` are each optional — a
 * WebRTC stack supplies one, the other, or both — and are OMITTED rather
 * than null when absent, like every optional field on this wire.
 */
@Serializable
data class IceCandidateDto(
    val candidate: String,
    @SerialName("sdp_mid") val sdpMid: String? = null,
    @SerialName("sdp_mline_index") val sdpMlineIndex: Int? = null,
)

/**
 * One STUN or TURN server from `GET /calls/ice` (docs/protocol.md,
 * `IceServer`). Credentials on a TURN server only, and only when the
 * operator configured them.
 */
@Serializable
data class IceServerDto(
    val urls: List<String>,
    val username: String? = null,
    val credential: String? = null,
)

/** `GET /calls/ice` — fetched at the start of every call, never cached. */
@Serializable
data class IceServersResponse(
    @SerialName("ice_servers") val iceServers: List<IceServerDto> = emptyList(),
    @SerialName("ttl_secs") val ttlSecs: Long = 0,
)

/**
 * A photo or video on a message (docs/protocol.md, "Photos, videos and files").
 *
 * Immutable once sent, with one exception: [hasPreview] flips from false
 * to true when the sender's preview upload lands, which may be after the
 * message itself.
 *
 * Dimensions are optional — the uploader may not have been able to work
 * them out — so a bubble that needs a shape before the bytes arrive uses
 * [aspectRatio], which has an answer either way.
 */
@Serializable
data class AttachmentDto(
    val id: Long,
    /** "photo" | "video". */
    val kind: String,
    val mime: String,
    val size: Long,
    val width: Int? = null,
    val height: Int? = null,
    @SerialName("duration_ms") val durationMs: Int? = null,
    @SerialName("has_preview") val hasPreview: Boolean = false,
    /**
     * Required on a file, where it is the attachment's whole identity — a
     * photo renders itself, where "attachment 34" tells nobody anything.
     * Optional on audio and on a location, where it is a label.
     */
    val name: String? = null,
    /**
     * Locations only, and always both: a location IS its coordinates
     * (docs/protocol.md, "Locations"). They ride on the attachment rather
     * than in bytes so a bubble can draw the pin the moment the message
     * arrives, without a download that might fail.
     */
    val latitude: Double? = null,
    val longitude: Double? = null,
    /**
     * Locations only, and only when the sending device knew one: the radius
     * in metres it believed the fix good to. Null means UNKNOWN, drawn as a
     * plain pin rather than as perfect precision.
     */
    @SerialName("accuracy_m") val accuracyM: Int? = null,
) {
    val isVideo: Boolean get() = kind == KIND_VIDEO
    val isFile: Boolean get() = kind == KIND_FILE
    val isAudio: Boolean get() = kind == KIND_AUDIO
    val isLocation: Boolean get() = kind == KIND_LOCATION

    /**
     * A filename for something that carries no name of its own.
     *
     * The EXTENSION is the part that matters: it is what the gallery, the
     * share target and the receiving app all read to decide what they have
     * been handed. Mirrors iOS's ChatSyncCoordinator.fallbackName.
     */
    val fallbackFileName: String
        get() {
            val ext = when (mime) {
                "image/jpeg" -> "jpg"
                "image/png" -> "png"
                "image/heic" -> "heic"
                "image/heif" -> "heif"
                "video/mp4" -> "mp4"
                "video/quicktime" -> "mov"
                else -> if (isVideo) "mp4" else "jpg"
            }
            return "${if (isVideo) "video" else "photo"}-$id.$ext"
        }

    /** What a bubble calls it: the name for a file, a word for the rest. */
    val displayName: String
        get() = name?.takeIf { it.isNotEmpty() }
            ?: when {
                isVideo -> "Video"
                isAudio -> "Audio"
                isLocation -> "Location"
                isFile -> "File"
                else -> "Photo"
            }

    /**
     * Width over height, or 4:3 when the uploader could not say. Reserving
     * the right shape before the image arrives is what stops a photo from
     * shoving the thread when it finishes loading.
     */
    val aspectRatio: Float
        get() {
            val w = width ?: return DEFAULT_ASPECT
            val h = height ?: return DEFAULT_ASPECT
            return if (w > 0 && h > 0) w.toFloat() / h.toFloat() else DEFAULT_ASPECT
        }

    companion object {
        const val KIND_PHOTO = "photo"
        const val KIND_VIDEO = "video"
        const val KIND_AUDIO = "audio"
        const val KIND_FILE = "file"
        const val KIND_LOCATION = "location"
        const val DEFAULT_ASPECT = 4f / 3f

        /**
         * Most attachments one message may carry (protocol.md, "Limits":
         * `limits.max_attachments_per_message`, whose fewest is 1, fixed).
         * The staging cap, the picker cap and the share-target cap all
         * read this one number.
         */
        const val MAX_PER_MESSAGE = 10
    }
}

@Serializable
data class AttachmentResponse(val attachment: AttachmentDto)

/**
 * Local persistence codec: the messages table stores a message's
 * reactions verbatim as the wire-shape JSON array (`reactionsJson`
 * column — null = never reacted, "[]" = cleared). Private Json so a
 * house-config change can never silently re-shape stored rows.
 */
object ReactionsCodec {
    private val json = Json { ignoreUnknownKeys = true }

    fun encode(reactions: List<ReactionDto>): String = json.encodeToString(reactions)

    fun decode(raw: String?): List<ReactionDto> =
        raw?.let { runCatching { json.decodeFromString<List<ReactionDto>>(it) }.getOrNull() }
            .orEmpty()
}

/**
 * Local persistence codec: the messages table stores a poll verbatim in
 * its WIRE shape (`pollJson` column - null = not a poll), beside the
 * `pollSeq` column that guards every apply.
 *
 * The Json instance is a static val, like [ReactionsCodec]'s, and for a
 * measured reason: allocating one per read cost real time when reactions
 * did it, and a poll is decoded for every visible bubble that has one.
 * Private, so a change to the house Json can never silently re-shape
 * stored rows.
 */
object PollCodec {
    private val json = Json { ignoreUnknownKeys = true }

    fun encode(poll: PollDto): String = json.encodeToString(poll)

    /** Null for a message that is not a poll, or for a row we cannot read. */
    fun decode(raw: String?): PollDto? =
        raw?.let { runCatching { json.decodeFromString<PollDto>(it) }.getOrNull() }
}

/**
 * Local persistence codec: the messages table stores a message's
 * attachments verbatim as the wire-shape JSON array (`attachmentsJson`
 * column — null = none stored there; rows written before plurality keep
 * their twelve flat columns and decode through the entity's fallback).
 * The same shape and the same private-Json reasoning as [PollCodec]:
 * replaced whole, never patched, and a house-config change can never
 * silently re-shape stored rows.
 */
object AttachmentsCodec {
    private val json = Json { ignoreUnknownKeys = true }

    fun encode(attachments: List<AttachmentDto>): String = json.encodeToString(attachments)

    /** Null when the column is null or unreadable — the caller then falls
     *  back to the flat columns a pre-plurality row still carries. */
    fun decode(raw: String?): List<AttachmentDto>? =
        raw?.let { runCatching { json.decodeFromString<List<AttachmentDto>>(it) }.getOrNull() }
}

// -- Request bodies ----------------------------------------------------------

@Serializable
data class RegisterRequest(
    val username: String,
    @SerialName("display_name") val displayName: String,
    val password: String,
)

@Serializable
data class LoginRequest(
    val username: String,
    val password: String,
)

@Serializable
data class CreateFamilyRequest(val name: String)

@Serializable
data class JoinFamilyRequest(@SerialName("invite_code") val inviteCode: String)

/**
 * PATCH /families/mine. Which fields are PRESENT decides what changes, so
 * an untouched one must be OMITTED — encodeDefaults=false in the house
 * Json is what does that.
 *
 * [language] is the exception the protocol calls out, and the only place
 * in it where a `null` means something a missing key does not: an omitted
 * key leaves the family's language alone, while an explicit JSON `null`
 * CLEARS it. A Kotlin `null` cannot say that here — it is the default, so
 * it is exactly what gets dropped — which is why the field is a
 * [JsonElement] and clearing sends [JsonNull]. The two cases are
 * indistinguishable in Kotlin's types and total on the wire, hence the
 * factories below rather than a raw constructor at each call site.
 */
@Serializable
data class PatchFamilyRequest(
    @SerialName("join_policy") val joinPolicy: String? = null,
    val language: JsonElement? = null,
    @SerialName("ai_history") val aiHistory: Boolean? = null,
    @SerialName("max_members") val maxMembers: JsonElement? = null,
) {
    companion object {
        fun joinPolicy(policy: String) = PatchFamilyRequest(joinPolicy = policy)

        /**
         * A number SETS the cap; null CLEARS it, as a real JSON null.
         *
         * `JsonElement` for the same reason `language` uses it: these are
         * the TWO places in the protocol where sending a `null` means
         * something a missing key does not, and `encodeDefaults=false`
         * would otherwise drop the clear entirely and make it a no-op
         * (docs/protocol.md, `PATCH /families/mine`).
         */
        fun maxMembers(cap: Int?) =
            PatchFamilyRequest(maxMembers = cap?.let(::JsonPrimitive) ?: JsonNull)

        /** A tag SETS the language; null CLEARS it, as a real JSON null. */
        fun language(tag: String?) =
            PatchFamilyRequest(language = tag?.let(::JsonPrimitive) ?: JsonNull)

        fun aiHistory(enabled: Boolean) = PatchFamilyRequest(aiHistory = enabled)
    }
}

/** PUT /me/birthday and PUT /families/members/{id}/birthday — same body. */
@Serializable
data class BirthdayRequest(
    val month: Int,
    val day: Int,
)

@Serializable
data class CreateDirectChatRequest(@SerialName("user_id") val userId: Long)

@Serializable
data class SendMessageRequest(
    @SerialName("client_msg_id") val clientMsgId: String,
    val body: String,
    // encodeDefaults=false in the house Json config, so a null is omitted
    // rather than sent as "reply_to_message_id": null — which is what the
    // protocol writes for an ordinary message.
    @SerialName("reply_to_message_id") val replyToMessageId: Long? = null,
    /**
     * The uploaded attachments this message claims, in the sender's order
     * (1-10). The PLURAL spelling always — `attachment_id` is the legacy
     * one-element form, still accepted but no longer sent.
     */
    @SerialName("attachment_ids") val attachmentIds: List<Long>? = null,
    /**
     * Makes this message a poll: the body is then the QUESTION and must
     * be non-empty, and `poll` and `attachment_ids` are mutually
     * exclusive (docs/protocol.md, "Polls"). Omitted for an ordinary
     * message — encodeDefaults=false again.
     */
    val poll: NewPollDto? = null,
)

/**
 * The options a new poll is created with — the ONLY poll shape a client
 * ever sends.
 *
 * Deliberately not a [PollDto]: a client has no say over ids, votes,
 * `closed` or the sequence, and a type that could express them would
 * invite one to try. The server's rules (2-10 options, each trimmed,
 * non-empty, at most 100 characters, no two the same ignoring case) are
 * mirrored in [me.nettrash.familyconnect.ui.chat.PollDraft] so the
 * composer can refuse before the round trip rather than after it.
 */
@Serializable
data class NewPollDto(val options: List<String>)

/** PUT /chats/{id}/messages/{mid}/vote — the option the caller now holds. */
@Serializable
data class VoteRequest(@SerialName("option_id") val optionId: Long)

@Serializable
data class EditMessageRequest(val body: String)

@Serializable
data class ChangePasswordRequest(
    @SerialName("current_password") val currentPassword: String,
    @SerialName("new_password") val newPassword: String,
)

@Serializable
data class ResetPasswordRequest(
    @SerialName("new_password") val newPassword: String,
)

/**
 * POST /me/delete — the body of an account deletion.
 *
 * The password is required for the same reason [ChangePasswordRequest]
 * carries one: a live session is not proof of who is holding the phone
 * (docs/protocol.md, "Deleting an account"). A POST rather than a
 * `DELETE /me` because the request carries a body.
 */
@Serializable
data class DeleteAccountRequest(val password: String)

/**
 * One sticker note on the family board.
 *
 * A TOMBSTONE is the same object with `deleted: true` and no content: the
 * change feed has to be able to say "this note is gone", and an absent row
 * cannot say anything (docs/protocol.md, "Board"). Every content field is
 * therefore nullable.
 */
@Serializable
data class NoteDto(
    val id: Long,
    @SerialName("author_id") val authorId: Long? = null,
    val text: String? = null,
    val color: String? = null,
    /**
     * "small" | "medium" | "large" — a STEP each client draws at its own
     * idiom, not a measurement (docs/protocol.md, "Board"). Null from a
     * server that predates the field, which means "medium": the size
     * every note had before there was one.
     */
    val size: String? = null,
    val x: Double? = null,
    val y: Double? = null,
    @SerialName("created_at") val createdAt: String? = null,
    @SerialName("updated_at") val updatedAt: String? = null,
    @SerialName("board_seq") val boardSeq: Long,
    /**
     * The board_seq of the last change to what the note SAYS — the only
     * number the badge counts (docs/protocol.md, "Board"). A move, a resize
     * and a recolour leave it where it was. Null on a tombstone and from a
     * server that predates the field, which is why it is nullable rather
     * than 0: "nobody said" is not "written at the dawn of the board".
     */
    @SerialName("content_seq") val contentSeq: Long? = null,
    val deleted: Boolean? = null,
) {
    val isTombstone: Boolean get() = deleted == true
}

@Serializable
data class BoardResponse(
    val notes: List<NoteDto>,
    @SerialName("max_board_seq") val maxBoardSeq: Long,
)

@Serializable
data class BoardChangesResponse(val notes: List<NoteDto>)

@Serializable
data class NoteResponse(val note: NoteDto)

@Serializable
data class CreateNoteRequest(
    val text: String,
    val color: String,
    val size: String,
    val x: Double,
    val y: Double,
)

/**
 * Every field optional: a MOVE sends only x/y (any member may), an edit
 * sends text, color and/or size (author only). Which fields are present is
 * what decides the permission the server applies — so nulls must be
 * OMITTED, which the house Json does via encodeDefaults=false.
 */
@Serializable
data class PatchNoteRequest(
    val text: String? = null,
    val color: String? = null,
    val size: String? = null,
    val x: Double? = null,
    val y: Double? = null,
)

@Serializable
data class ReadRequest(@SerialName("last_read_message_id") val lastReadMessageId: Long)

@Serializable
data class ReactionRequest(val emoji: String)

@Serializable
data class DeviceRequest(
    val platform: String,
    // No default so `push_token: null` is serialized explicitly, exactly
    // as the protocol table writes it. Null when the build has no FCM
    // config (no google-services.json) — the device row still registers,
    // it just can't be pushed to.
    @SerialName("push_token") val pushToken: String?,
)

// -- Response envelopes -------------------------------------------------------

@Serializable
data class AuthResponse(
    val token: String,
    val user: UserDto,
)

@Serializable
data class MeResponse(
    val user: UserDto,
    val family: FamilyDto? = null,
    val role: String? = null,
    @SerialName("pending_join_request") val pendingJoinRequest: PendingJoinRequestDto? = null,
    /**
     * Whether this server signals voice calls at all (`[calls] enabled`).
     * ALWAYS present on a current server; the default covers one that
     * predates calls, which then — correctly — shows no call button.
     */
    @SerialName("calls_enabled") val callsEnabled: Boolean = false,
    /**
     * Whether this server allows VIDEO calls (`[calls] video_enabled`,
     * docs/protocol.md, "Video"). Gates the video-call button ALONE —
     * voice is untouched; the default covers a server that predates
     * video, which then correctly offers voice only.
     */
    @SerialName("video_calls_enabled") val videoCallsEnabled: Boolean = false,
    /**
     * The operator's ceiling on a family's size
     * (`limits.max_family_members`). An owner's cap picker draws its range
     * from this instead of discovering `validation` at the moment somebody
     * tries to set one.
     *
     * ALWAYS present on a current server. Null means one too old to say,
     * and the cap control hides rather than inventing a bound.
     */
    @SerialName("max_family_members") val maxFamilyMembers: Int? = null,
    /**
     * The caller's own block list. ALWAYS present and `[]` when they have
     * blocked nobody — the one read in this protocol where absence is not
     * allowed to mean "leave what you hold alone", because a list that
     * vanished when it emptied would never tell a second device about the
     * last unblock. A complete state-set and never a delta, so a client
     * REPLACES what it stores (docs/protocol.md, "Blocking a member").
     *
     * It rides here as well as on `GET /families/mine` because a block is
     * a pair and not a membership: a caller with no family at all still
     * holds blocks.
     */
    @SerialName("blocked_user_ids") val blockedUserIds: List<Long> = emptyList(),
    /**
     * The operator's published contact (`[server] support_contact`),
     * absent when unset. Shown on the report screen.
     *
     * Free text, at most 256 characters, in whatever form the operator
     * configured. Clients draw it VERBATIM, selectable and copyable, and
     * NEVER linkify it — an operator may write an address, a URL or a
     * sentence, and three apps guessing differently about which it is
     * would be worse than three apps showing the same text
     * (docs/protocol.md, `GET /me`).
     */
    @SerialName("support_contact") val supportContact: String? = null,
)

@Serializable
data class JoinResponse(val status: String)

/** PUT /me/avatar — the caller's user with its bumped avatar_version. */
@Serializable
data class AvatarResponse(val user: UserDto)

/** PUT /me/birthday — the caller's user, carrying what was just stored. */
@Serializable
data class BirthdayResponse(val user: UserDto)

/**
 * PUT /families/members/{id}/birthday — the member the owner just wrote.
 *
 * The response is the ONLY way the writing device learns the new value
 * without a resync: a birthday change raises no WebSocket frame and no
 * push (docs/protocol.md, "Birthdays").
 */
@Serializable
data class MemberBirthdayResponse(val member: MemberDto)

@Serializable
data class FamilyResponse(val family: FamilyDto)

@Serializable
data class FamilyMineResponse(
    val family: FamilyDto,
    val members: List<MemberDto>,
    /**
     * The accounts that were DELETED while in this family — each with
     * `deleted: true` and no `role`, omitted entirely when there are none
     * (docs/protocol.md, "Deleting an account").
     *
     * They are not members: nothing counts them, nothing offers them, and
     * every roster on this client filters them out. They exist so a stored
     * message, note or reaction can still be given a name. A client stores
     * both arrays in ONE place and draws only [members].
     */
    @SerialName("former_members") val formerMembers: List<MemberDto> = emptyList(),
    // The board cursor, omitted while the board has never been written to.
    @SerialName("max_board_seq") val maxBoardSeq: Long? = null,
    // Absent when the server has no assistant configured, which is the
    // whole of the capability check (docs/protocol.md, "Mentioning the
    // assistant in the family chat").
    val assistant: AssistantDto? = null,
    /**
     * Everybody the CALLER has blocked. A client REPLACES what it stores
     * with this rather than merging (docs/protocol.md, "Blocking a
     * member"); the empty default is right both for a server that
     * predates blocking and for a caller who has blocked nobody.
     */
    @SerialName("blocked_user_ids") val blockedUserIds: List<Long> = emptyList(),
    /**
     * Who would inherit this family if the owner left right now — present
     * for the OWNER only, and absent when they are the last member, which
     * is a DIFFERENT dialog: leaving then deletes the family.
     *
     * A PREDICTION, not a fact. Any join or leave changes the answer and
     * none of them raises a frame for it, so this is only ever read
     * straight after a fresh `GET /families/mine` and never from a cached
     * copy (docs/protocol.md, `GET /families/mine`).
     */
    @SerialName("next_owner_user_id") val nextOwnerUserId: Long? = null,
)

/**
 * The body of a `POST /families/leave` that handed the family on. The
 * endpoint answers `204` with NO body at all when nothing passed on — an
 * ordinary member leaving, or the last one, who takes the family with
 * them — so this type must never be required for the call to succeed.
 */
@Serializable
data class LeaveFamilyResponse(
    @SerialName("new_owner_user_id") val newOwnerUserId: Long? = null,
)

/**
 * The assistant, as `GET /families/mine` reports it.
 *
 * Not a [MemberDto] and deliberately not in `members`: it belongs to no
 * family, cannot be messaged one-to-one, removed, made owner or given a
 * password, and every screen that lists people would need a special case
 * for it. What it IS good for is naming its messages in the family chat —
 * its reserved account is absent from the roster on purpose, so a lookup
 * there finds nothing — and telling the composer the feature exists.
 */
@Serializable
data class AssistantDto(
    @SerialName("user_id") val userId: Long,
    @SerialName("display_name") val displayName: String,
    /**
     * The token that summons it, from the server rather than hard-coded:
     * the grammar is shared ([me.nettrash.familyconnect.ui.chat.AssistantMention])
     * but the spelling belongs to the protocol, and a client inventing its
     * own would be unanswerable.
     */
    val mention: String,
)

@Serializable
data class RotateInviteCodeResponse(@SerialName("invite_code") val inviteCode: String)

@Serializable
data class JoinRequestsResponse(val requests: List<JoinRequestDto>)

@Serializable
data class ApproveResponse(val member: MemberDto)

/**
 * One report in the owner's moderation list (docs/protocol.md,
 * "Reporting a member").
 */
@Serializable
data class ReportDto(
    val id: Long,
    val reporter: UserDto,
    val reported: UserDto,
    /**
     * One of `spam`, `harassment`, `inappropriate`, `other` — a fixed list
     * so nine locales render a row from string resources rather than
     * shipping untranslated prose to a moderator. An unrecognised value is
     * KEPT as-is and drawn as "other" rather than failing the decode: a
     * newer server must never make an owner's inbox unreadable.
     */
    val reason: String,
    /**
     * The message reported, when one was named AND it still exists.
     * Retention drops it; the excerpt outlives it, so a client draws the
     * excerpt always and offers to jump to the message only while this
     * survives.
     */
    @SerialName("message_id") val messageId: Long? = null,
    /**
     * The WHOLE reported body, frozen when the report was raised — the one
     * quotation in this protocol that is STORED rather than recomputed,
     * because the author may edit it away and retention will sweep it.
     */
    @SerialName("message_excerpt") val messageExcerpt: String? = null,
    @SerialName("created_at") val createdAt: String? = null,
)

@Serializable
data class ReportsResponse(val reports: List<ReportDto>)

@Serializable
data class ReportResponse(val report: ReportDto)

/**
 * `POST /families/reports`. [messageId] names one message and is omitted
 * for a report about the PERSON; `encodeDefaults=false` in the house Json
 * config is what keeps it off the wire rather than sending an explicit
 * null.
 */
@Serializable
data class CreateReportRequest(
    @SerialName("reported_user_id") val reportedUserId: Long,
    val reason: String,
    @SerialName("message_id") val messageId: Long? = null,
)

@Serializable
data class ChatListItemDto(
    val chat: ChatDto,
    @SerialName("last_message") val lastMessage: MessageDto? = null,
    @SerialName("unread_count") val unreadCount: Int,
    /**
     * The CALLER'S OWN read marker for this chat — what `POST
     * /chats/{id}/read` and the `read` frame maintain, monotonic and
     * shared across every device this person owns. The other half of
     * [unreadCount], off the same row of the same query, so the two
     * always describe one instant.
     *
     * Unlike the three `max_*_seq` cursors below, the protocol says it is
     * ALWAYS present and that `0` is a real answer — "has never reported
     * reading anything here" — rather than an absent one. Nullable here
     * for exactly one reason: a server binary older than the field omits
     * it and this client must go on reading its chat list. Nothing else
     * has to care, because the marker is applied monotonically
     * (`max(stored, received)`), so absent lands in the same place as 0 —
     * on the stored value, unchanged.
     *
     * An id THRESHOLD and never a reference: retention may already have
     * swept the message it names, so nothing may try to fetch it.
     */
    @SerialName("last_read_message_id") val lastReadMessageId: Long? = null,
    // Absent while no message in the chat has ever been reacted to.
    @SerialName("max_reaction_seq") val maxReactionSeq: Long? = null,
    // Absent while nothing in the chat has been edited.
    @SerialName("max_edit_seq") val maxEditSeq: Long? = null,
    // Absent while the chat holds no poll at all. The fourth cursor of
    // the same shape as the two above, for the same reason: `after_id`
    // can never see a change to an older row, and a vote is nothing but
    // a change to an older row.
    @SerialName("max_poll_seq") val maxPollSeq: Long? = null,
)

@Serializable
data class ChatsResponse(val chats: List<ChatListItemDto>)

@Serializable
data class ChatResponse(val chat: ChatDto)

@Serializable
data class MessagesResponse(val messages: List<MessageDto>)

@Serializable
data class MessageResponse(val message: MessageDto)

/**
 * One message's full reaction state — the PUT/DELETE reaction response
 * AND each entry of the GET /chats/{id}/reactions catch-up (protocol:
 * the same shape on purpose; frames carry it too, never a delta).
 */
@Serializable
data class MessageReactionStateDto(
    @SerialName("message_id") val messageId: Long,
    @SerialName("reaction_seq") val reactionSeq: Long,
    val reactions: List<ReactionDto>,
)

@Serializable
data class ReactionsCatchUpResponse(
    @SerialName("message_reactions") val messageReactions: List<MessageReactionStateDto>,
)

/**
 * One message's full poll state - the vote / retract / close response
 * AND each entry of the `GET /chats/{id}/polls` catch-up. The same shape
 * on purpose, exactly as the reaction pair above: frames carry it too,
 * and never a delta.
 */
@Serializable
data class MessagePollStateDto(
    @SerialName("message_id") val messageId: Long,
    val poll: PollDto,
)

@Serializable
data class PollsCatchUpResponse(val polls: List<MessagePollStateDto>)

@Serializable
data class DeviceResponse(@SerialName("device_id") val deviceId: Long)

// -- Error shape ---------------------------------------------------------------

@Serializable
data class ErrorBody(val error: ErrorPayload) {
    @Serializable
    data class ErrorPayload(
        val code: String,
        val message: String,
    )
}

// -- Family statistics --------------------------------------------------------

/**
 * What the family has actually sent (protocol.md, "Family statistics").
 * Every member sees the same numbers.
 */
@Serializable
data class FamilyStatsDto(
    val totals: StatsTotalsDto,
    val members: List<MemberStatsDto> = emptyList(),
)

@Serializable
data class StatsTotalsDto(
    val members: Int = 0,
    val messages: Int = 0,
    @SerialName("board_notes") val boardNotes: Int = 0,
    val attachments: AttachmentStatsDto = AttachmentStatsDto(),
    val ai: AiStatsDto = AiStatsDto(),
)

@Serializable
data class MemberStatsDto(
    @SerialName("user_id") val userId: Long,
    @SerialName("display_name") val displayName: String,
    val messages: Int = 0,
    val attachments: AttachmentStatsDto = AttachmentStatsDto(),
    val ai: AiStatsDto = AiStatsDto(),
)

@Serializable
data class AttachmentStatsDto(
    val count: Int = 0,
    val bytes: Long = 0,
    val photo: Int = 0,
    val video: Int = 0,
    val audio: Int = 0,
    val file: Int = 0,
    /**
     * Each distinct file counted once. Family totals ONLY — a file two
     * members both sent belongs to neither alone, so there is no per-member
     * share of it, and the field is absent on a member row.
     */
    @SerialName("stored_bytes") val storedBytes: Long? = null,
)

@Serializable
data class AiStatsDto(
    val questions: Int = 0,
    @SerialName("prompt_tokens") val promptTokens: Int = 0,
    @SerialName("completion_tokens") val completionTokens: Int = 0,
)
