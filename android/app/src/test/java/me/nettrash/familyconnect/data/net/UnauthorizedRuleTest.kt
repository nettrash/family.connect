/*
 * UnauthorizedRuleTest.kt
 * Family Connect (Android)
 *
 * Which 401 means "the session is gone" (docs/protocol.md, "Auth") — the
 * decision that wipes local state and returns the app to the sign-in
 * screen.
 *
 * Two opposite mistakes are possible and this app had made the second
 * one. Missing a real dead session leaves a phone showing a family it can
 * no longer reach — that is how every OTHER device of a deleted account
 * is supposed to find out. Treating a wrong PASSWORD as one signs
 * somebody out over a typo, on the two endpoints that ask for a password
 * while already holding a live session: `POST /me/password` and
 * `POST /me/delete`. Account deletion is what made the second one
 * unmissable, since getting the password wrong on that dialog logged the
 * user out of an account that still exists.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class UnauthorizedRuleTest {

    @Test
    fun `an authenticated unauthorized 401 is a dead session`() {
        assertThat(ApiClient.isSessionGone(401, "unauthorized", auth = true)).isTrue()
    }

    @Test
    fun `a 401 with no parseable error body is still a dead session`() {
        // A proxy's own 401 page, say. "The server would not say" is not a
        // reason to go on believing in the session.
        assertThat(ApiClient.isSessionGone(401, null, auth = true)).isTrue()
    }

    @Test
    fun `invalid credentials is a wrong password, not a dead session`() {
        assertThat(ApiClient.isSessionGone(401, "invalid_credentials", auth = true)).isFalse()
    }

    @Test
    fun `an unauthenticated 401 never counts`() {
        // Login, register and the server-setup probe 401 routinely — the
        // probe treats it as the SUCCESS signal.
        assertThat(ApiClient.isSessionGone(401, "invalid_credentials", auth = false)).isFalse()
        assertThat(ApiClient.isSessionGone(401, "unauthorized", auth = false)).isFalse()
    }

    @Test
    fun `no other status counts`() {
        assertThat(ApiClient.isSessionGone(403, "not_family_owner", auth = true)).isFalse()
        assertThat(ApiClient.isSessionGone(404, "not_found", auth = true)).isFalse()
        assertThat(ApiClient.isSessionGone(500, null, auth = true)).isFalse()
    }
}
