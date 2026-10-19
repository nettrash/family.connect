/*
 * LocalNetworkAccess.kt
 * Family Connect (Android)
 *
 * Android 17's local network permission, for an app that targets it (API
 * 37). ACCESS_LOCAL_NETWORK is a runtime permission (the Nearby devices
 * group), and without it every connection to an address on the device's
 * own Wi-Fi or Ethernet network — TCP and UDP, OkHttp and libwebrtc alike —
 * is dropped: TCP connects simply time out. Two things here go there:
 *
 *   - the family's server, when it runs on the home network
 *     (http://192.168.1.10:8080, nas.local) — the self-hosted case the
 *     network security config's cleartext base exists for. Without the
 *     grant the app could not reach it at all;
 *   - a call between two people on the same Wi-Fi, whose shortest ICE path
 *     is a pair of LAN addresses. Without the grant the call still connects
 *     over the internet path, if the router or a relay allows one.
 *
 * Internet addresses need nothing, so a family on a public server is never
 * asked about the server; and on Android 16 and older the permission does
 * not exist (INTERNET is enough), so nothing here ever asks there.
 *
 * Which addresses count is the platform's: the private, shared (CGNAT) and
 * link-local IPv4 ranges, IPv6 link-local and unique-local, multicast and
 * broadcast, and `.local` names, which only resolve by multicast DNS.
 */

package me.nettrash.familyconnect.data.net

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress

object LocalNetworkAccess {

    /**
     * `Manifest.permission.ACCESS_LOCAL_NETWORK`, spelled out: the constant
     * exists only from API 37, and every use is behind [needsGrant].
     */
    const val PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

    /** Asked once per process at most, whoever asks first — never twice in a row. */
    @Volatile
    private var asked = false

    /**
     * This device enforces the permission (Android 17+), the app TARGETS 37 —
     * the platform holds an app to it only then, and targetSdk stays at 36
     * until the 1.1-style video compress is checked on a real Android 17
     * phone (build.gradle.kts) — and the app does not hold it.
     */
    fun needsGrant(context: Context): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.CINNAMON_BUN &&
            context.applicationInfo.targetSdkVersion >= Build.VERSION_CODES.CINNAMON_BUN &&
            ContextCompat.checkSelfPermission(context, PERMISSION) != PackageManager.PERMISSION_GRANTED

    /**
     * Whether to show the system prompt now: the grant is missing and nobody
     * has asked yet in this process. Claims the one ask when it says yes.
     */
    @Synchronized
    fun claimAsk(context: Context): Boolean {
        if (asked || !needsGrant(context)) return false
        asked = true
        return true
    }

    /**
     * Whether the server at [serverUrl] (a normalized URL) is on the local
     * network. Resolves a host name — DNS itself is exempt from the
     * permission — so off the main thread.
     */
    suspend fun serverIsLocal(serverUrl: String): Boolean {
        val host = serverUrl.toHttpUrlOrNull()?.host ?: return false
        hostIsLocalWithoutLookup(host)?.let { return it }
        return withContext(Dispatchers.IO) {
            runCatching { InetAddress.getAllByName(host).any(::isLocal) }
                // Not resolvable here: a bare name ("nas") is somebody's
                // own network; a dotted one that does not resolve is not.
                .getOrElse { '.' !in host }
        }
    }

    /**
     * The answer when [host] alone gives it — an IP literal, `localhost`, a
     * `.local` name — or null when only a lookup can tell.
     */
    fun hostIsLocalWithoutLookup(host: String): Boolean? {
        val bare = host.trim('[', ']').lowercase()
        return when {
            bare == "localhost" -> false
            bare.endsWith(".local") -> true
            isIpLiteral(bare) -> isLocal(InetAddress.getByName(bare))
            else -> null
        }
    }

    /** An address the platform treats as local network traffic. Loopback is not. */
    fun isLocal(address: InetAddress): Boolean {
        if (address.isLoopbackAddress) return false
        if (address.isMulticastAddress || address.isLinkLocalAddress || address.isSiteLocalAddress) return true
        val b = address.address.map { it.toInt() and 0xFF }
        return when (address) {
            // 100.64.0.0/10 (shared address space) and the limited broadcast.
            is Inet4Address -> (b[0] == 100 && b[1] in 64..127) || b.all { it == 255 }
            // fc00::/7, unique local.
            is Inet6Address -> (b[0] and 0xFE) == 0xFC
            else -> false
        }
    }

    // A literal never triggers a lookup in InetAddress.getByName; a name
    // would, so only a literal may be passed to it on the caller's thread.
    private fun isIpLiteral(host: String): Boolean =
        ':' in host || IPV4_LITERAL.matches(host)

    private val IPV4_LITERAL = Regex("""\d{1,3}(\.\d{1,3}){3}""")
}
