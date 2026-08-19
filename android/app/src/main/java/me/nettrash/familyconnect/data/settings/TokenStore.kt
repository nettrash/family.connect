/*
 * TokenStore.kt
 * Family Connect (Android)
 *
 * Persistence boundary for the opaque session token. The token is
 * encrypted under an AES-256/GCM key held inside Android Keystore and
 * the (iv, ciphertext) pair lands in plain SharedPreferences — the same
 * hand-rolled wrap-with-keystore pattern as exchange-android's
 * IdentityStore.
 *
 * Why not `EncryptedSharedPreferences`: it was deprecated in
 * androidx.security 1.1.0, and pulling a deprecated crypto shim in for a
 * single secret isn't worth the maintenance debt. Keystore keys live in
 * TEE / StrongBox on supported devices, so an attacker who lifts the raw
 * disk can't read the session without code execution on the device.
 *
 * Interface + impl split so JVM tests can substitute an in-memory fake —
 * Robolectric has no AndroidKeyStore provider.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Settings/TokenStore.swift
 * (Keychain-backed there).
 */

package me.nettrash.familyconnect.data.settings

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

interface TokenStore {
    /** The stored session token, or null when logged out / undecryptable. */
    fun load(): String?

    fun save(token: String)

    fun clear()
}

class KeystoreTokenStore(context: Context) : TokenStore {

    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    @Synchronized
    override fun load(): String? {
        val ivB64 = prefs.getString(KEY_IV, null) ?: return null
        val ctB64 = prefs.getString(KEY_CIPHERTEXT, null) ?: return null
        return try {
            val iv = android.util.Base64.decode(ivB64, android.util.Base64.NO_WRAP)
            val ct = android.util.Base64.decode(ctB64, android.util.Base64.NO_WRAP)
            String(decryptWithKeystore(iv, ct), Charsets.UTF_8)
        } catch (_: Exception) {
            // Undecryptable (key rotated by OS, restored-from-backup prefs
            // without the key, …) — treat as logged out; the 401 path
            // would have landed us there anyway.
            null
        }
    }

    @Synchronized
    override fun save(token: String) {
        val (iv, ct) = encryptWithKeystore(token.toByteArray(Charsets.UTF_8))
        prefs.edit()
            .putString(KEY_IV, android.util.Base64.encodeToString(iv, android.util.Base64.NO_WRAP))
            .putString(KEY_CIPHERTEXT, android.util.Base64.encodeToString(ct, android.util.Base64.NO_WRAP))
            .apply()
    }

    @Synchronized
    override fun clear() {
        prefs.edit().remove(KEY_IV).remove(KEY_CIPHERTEXT).apply()
    }

    // -- Internals ----------------------------------------------------------

    private fun encryptWithKeystore(plain: ByteArray): Pair<ByteArray, ByteArray> {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateMasterKey())
        val ct = cipher.doFinal(plain)
        return cipher.iv to ct
    }

    private fun decryptWithKeystore(iv: ByteArray, ct: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            getOrCreateMasterKey(),
            GCMParameterSpec(GCM_TAG_BITS, iv),
        )
        return cipher.doFinal(ct)
    }

    private fun getOrCreateMasterKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }

        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        val spec = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            .build()
        gen.init(spec)
        return gen.generateKey()
    }

    private companion object {
        const val PREFS_NAME = "familyconnect.session.v1"
        const val KEY_IV = "iv"
        const val KEY_CIPHERTEXT = "ct"
        const val KEY_ALIAS = "familyconnect-session-master"
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val GCM_TAG_BITS = 128
    }
}
