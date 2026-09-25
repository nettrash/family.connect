/*
 * NoteKindsTest.kt
 * Family Connect (Android) — tests
 *
 * What a note IS (docs/protocol.md, "Board"), and the forward-compat rule
 * that matters most: a kind this client has never heard of DRAWS AS TEXT
 * rather than being dropped. The note still has a slot on a wall the whole
 * family shares, and a hole in that layout is worse than a sticker showing
 * only what it says.
 *
 * iOS counterpart: NoteKind in Views/NoteKind.swift.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class NoteKindsTest {

    @Test
    fun `the vocabulary is the protocol's`() {
        assertThat(NoteKinds.TEXT).isEqualTo("text")
        assertThat(NoteKinds.PHOTO).isEqualTo("photo")
    }

    @Test
    fun `only photo is a photo, and an unknown kind is not`() {
        assertThat(NoteKinds.isPhoto("photo")).isTrue()
        assertThat(NoteKinds.isPhoto("text")).isFalse()
        // The one that matters: a kind from a NEWER server draws as text.
        assertThat(NoteKinds.isPhoto("event")).isFalse()
        assertThat(NoteKinds.isPhoto("")).isFalse()
        assertThat(NoteKinds.isPhoto("PHOTO")).isFalse()
    }
}
