/*
 * ServerUrlNormalizer.kt
 * Family Connect (Android)
 *
 * Pure functions that turn whatever the user typed into the canonical
 * server URL, and derive the REST base ({url}/api/v1) and the WebSocket
 * URL ({url}/api/v1/ws, http→ws / https→wss) from it. Kept free of
 * android.* so ServerUrlNormalizerTest runs on the plain JVM.
 *
 * Validation rides on OkHttp's HttpUrl — the same parser that will
 * eventually issue the requests, so "normalizes" and "connectable"
 * can't drift apart.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Settings/ServerUrlNormalizer.swift
 */

package me.nettrash.familyconnect.data.settings

import okhttp3.HttpUrl.Companion.toHttpUrlOrNull

object ServerUrlNormalizer {

    /**
     * Canonicalize raw user input: trim whitespace, default to https://
     * when no scheme was typed, strip any trailing slashes. Returns null
     * when the result is not a valid http(s) URL.
     */
    fun normalize(input: String): String? {
        var candidate = input.trim()
        if (candidate.isEmpty()) return null
        if (!candidate.startsWith("http://", ignoreCase = true) &&
            !candidate.startsWith("https://", ignoreCase = true)
        ) {
            // A bare host ("chat.example.com" / "192.168.1.10:8080")
            // defaults to https — the safe assumption; LAN users type
            // http:// explicitly and get the cleartext warning card.
            candidate = "https://$candidate"
        }
        candidate = candidate.trimEnd('/')
        val url = candidate.toHttpUrlOrNull() ?: return null
        // Rebuild from the parsed form so scheme/host casing is canonical
        // and default ports are dropped, then strip HttpUrl's trailing "/".
        return url.toString().trimEnd('/')
    }

    /** REST base: `{url}/api/v1` (docs/protocol.md "Transport"). */
    fun apiBase(normalizedUrl: String): String = "$normalizedUrl/api/v1"

    /** WebSocket URL: `{url}/api/v1/ws`, scheme mapped http→ws / https→wss. */
    fun wsUrl(normalizedUrl: String): String {
        val wsBase = when {
            normalizedUrl.startsWith("https://") -> "wss://" + normalizedUrl.removePrefix("https://")
            normalizedUrl.startsWith("http://") -> "ws://" + normalizedUrl.removePrefix("http://")
            else -> normalizedUrl
        }
        return "$wsBase/api/v1/ws"
    }

    /** True when the normalized URL is cleartext http — drives the UX warning. */
    fun isCleartext(normalizedUrl: String): Boolean =
        normalizedUrl.startsWith("http://")
}
