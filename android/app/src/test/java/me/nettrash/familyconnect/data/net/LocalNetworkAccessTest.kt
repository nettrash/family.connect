package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.coroutines.test.runTest
import org.junit.Test
import java.net.InetAddress

/**
 * Which server addresses Android 17's local network grant is asked for
 * (LocalNetworkAccess). Only literals and `.local` names here: nothing in
 * these tests may reach a DNS server.
 */
class LocalNetworkAccessTest {

    private fun local(literal: String) = LocalNetworkAccess.isLocal(InetAddress.getByName(literal))

    @Test
    fun `the private, shared and link-local IPv4 ranges are local`() {
        listOf("10.0.0.1", "172.16.0.1", "172.31.255.254", "192.168.1.10", "169.254.3.4", "100.64.0.1", "100.127.255.254")
            .forEach { assertWithMessage(it).that(local(it)).isTrue() }
    }

    @Test
    fun `multicast and broadcast are local`() {
        listOf("224.0.0.251", "255.255.255.255", "ff02::fb").forEach { assertWithMessage(it).that(local(it)).isTrue() }
    }

    @Test
    fun `IPv6 link-local and unique-local are local`() {
        listOf("fe80::1", "fd12:3456:789a::1", "fc00::1").forEach { assertWithMessage(it).that(local(it)).isTrue() }
    }

    @Test
    fun `public addresses and loopback are not`() {
        listOf("8.8.8.8", "172.32.0.1", "100.128.0.1", "100.63.255.255", "127.0.0.1", "::1", "2001:4860:4860::8888")
            .forEach { assertWithMessage(it).that(local(it)).isFalse() }
    }

    @Test
    fun `a host alone answers for literals, localhost and local names`() {
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("192.168.1.10")).isTrue()
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("[fe80::1]")).isTrue()
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("93.184.216.34")).isFalse()
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("localhost")).isFalse()
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("NAS.local")).isTrue()
        assertThat(LocalNetworkAccess.hostIsLocalWithoutLookup("chat.example.com")).isNull()
    }

    @Test
    fun `a server url is judged by its host`() = runTest {
        assertThat(LocalNetworkAccess.serverIsLocal("http://192.168.1.10:8080")).isTrue()
        assertThat(LocalNetworkAccess.serverIsLocal("https://family.local")).isTrue()
        assertThat(LocalNetworkAccess.serverIsLocal("https://1.1.1.1")).isFalse()
        assertThat(LocalNetworkAccess.serverIsLocal("not a url")).isFalse()
    }
}
