/*
 * RoundVideoLimits.kt
 * Family Connect (Android)
 *
 * Video messages' two limits, as `GET /families/mine` reports them (#79,
 * docs/audio-video-messages-2026-10-04.md, "Discovery and limits";
 * docs/protocol.md, "Video messages"): `max_round_video_ms`, always 60 000
 * today, and `max_round_video_bytes`, the operator's ceiling. Both are ALWAYS
 * present on a server that has video messages, so their absence is the whole
 * capability check — the sticker pack's precedent (PackLimits): a client
 * offers no video entry against a server without them and never sends
 * `round` there.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.settings.SettingsState

data class RoundVideoLimits(val maxMs: Long, val maxBytes: Long) {
    companion object {
        /**
         * The limits a server answered with, or null when it did not say
         * BOTH — one without the other is not a server that has them — or
         * said something no clip could meet.
         */
        fun of(maxMs: Long?, maxBytes: Long?): RoundVideoLimits? =
            if (maxMs != null && maxBytes != null && maxMs > 0 && maxBytes > 0) {
                RoundVideoLimits(maxMs, maxBytes)
            } else {
                null
            }
    }
}

/** What this device last heard; null — the stored 0s — is a server without video messages. */
val SettingsState.roundVideoLimits: RoundVideoLimits?
    get() = RoundVideoLimits.of(roundVideoMaxMs, roundVideoMaxBytes)
