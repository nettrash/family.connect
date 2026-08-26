package me.nettrash.familyconnect.data.net.ws

import com.google.common.truth.Truth.assertThat
import org.junit.Test

/**
 * The rule ChatSocketManager connects by: the foreground OR a call, and a
 * session that can chat in either case (docs/protocol.md, "Voice calls":
 * "a client keeps its socket open for the life of a call").
 */
class SocketDesireTest {

    @Test
    fun theForegroundWantsASocket() {
        assertThat(ChatSocketManager.socketDesired(foregrounded = true, inCall = false, canChat = true)).isTrue()
    }

    @Test
    fun theBackgroundDoesNot() {
        assertThat(ChatSocketManager.socketDesired(foregrounded = false, inCall = false, canChat = true)).isFalse()
    }

    @Test
    fun aCallHoldsTheSocketOpenInTheBackground() {
        // The push woke the phone; the offer arrives over the socket.
        assertThat(ChatSocketManager.socketDesired(foregrounded = false, inCall = true, canChat = true)).isTrue()
    }

    @Test
    fun nothingHoldsASocketForASessionThatCannotChat() {
        assertThat(ChatSocketManager.socketDesired(foregrounded = true, inCall = true, canChat = false)).isFalse()
        assertThat(ChatSocketManager.socketDesired(foregrounded = false, inCall = true, canChat = false)).isFalse()
    }
}
