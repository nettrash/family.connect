/*
 * ServerUrlNormalizerTest.kt
 * Family Connect (Android)
 *
 * The normalizer is the only thing between "whatever the user typed"
 * and every request the app ever makes — pinned tightly.
 */

package me.nettrash.familyconnect.data.settings

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ServerUrlNormalizerTest {

    @Test
    fun bareHostDefaultsToHttps() {
        assertThat(ServerUrlNormalizer.normalize("chat.example.com"))
            .isEqualTo("https://chat.example.com")
    }

    @Test
    fun whitespaceIsTrimmed() {
        assertThat(ServerUrlNormalizer.normalize("  chat.example.com  "))
            .isEqualTo("https://chat.example.com")
    }

    @Test
    fun trailingSlashesAreStripped() {
        assertThat(ServerUrlNormalizer.normalize("https://chat.example.com///"))
            .isEqualTo("https://chat.example.com")
    }

    @Test
    fun explicitHttpWithPortSurvives() {
        assertThat(ServerUrlNormalizer.normalize("http://192.168.1.10:8080"))
            .isEqualTo("http://192.168.1.10:8080")
    }

    @Test
    fun defaultPortsAreDropped() {
        assertThat(ServerUrlNormalizer.normalize("https://chat.example.com:443"))
            .isEqualTo("https://chat.example.com")
        assertThat(ServerUrlNormalizer.normalize("http://chat.example.com:80"))
            .isEqualTo("http://chat.example.com")
    }

    @Test
    fun schemeAndHostAreLowercased() {
        assertThat(ServerUrlNormalizer.normalize("HTTPS://Chat.Example.COM"))
            .isEqualTo("https://chat.example.com")
    }

    @Test
    fun emptyAndBlankInputAreRejected() {
        assertThat(ServerUrlNormalizer.normalize("")).isNull()
        assertThat(ServerUrlNormalizer.normalize("   ")).isNull()
    }

    @Test
    fun garbageIsRejected() {
        assertThat(ServerUrlNormalizer.normalize("ht tp://bad host")).isNull()
        assertThat(ServerUrlNormalizer.normalize("https://")).isNull()
    }

    @Test
    fun apiBaseAppendsApiV1() {
        assertThat(ServerUrlNormalizer.apiBase("https://chat.example.com"))
            .isEqualTo("https://chat.example.com/api/v1")
    }

    @Test
    fun wsUrlMapsHttpsToWss() {
        assertThat(ServerUrlNormalizer.wsUrl("https://chat.example.com"))
            .isEqualTo("wss://chat.example.com/api/v1/ws")
    }

    @Test
    fun wsUrlMapsHttpToWs() {
        assertThat(ServerUrlNormalizer.wsUrl("http://192.168.1.10:8080"))
            .isEqualTo("ws://192.168.1.10:8080/api/v1/ws")
    }

    @Test
    fun cleartextDetectionMatchesScheme() {
        assertThat(ServerUrlNormalizer.isCleartext("http://192.168.1.10:8080")).isTrue()
        assertThat(ServerUrlNormalizer.isCleartext("https://chat.example.com")).isFalse()
    }
}
