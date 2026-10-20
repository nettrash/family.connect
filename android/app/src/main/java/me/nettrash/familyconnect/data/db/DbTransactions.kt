/*
 * DbTransactions.kt
 * Family Connect (Android)
 *
 * The one seam through which a repository commits several writes as one
 * database transaction.
 *
 * Why the message store needs it: a page of history or catch-up used to be
 * applied a row at a time, each write its own implicit transaction — its own
 * commit, and its own invalidation of every query watching the messages
 * table. Opening a chat after a long absence means hundreds of rows arriving
 * that way, and every one of them made the open chat re-run its window query
 * and rebuild every row, while other chats' catch-ups (same table) did the
 * same to it. Committed as one transaction a page is ONE change: one commit
 * and one emission, carrying the whole page — and the rows it writes, and
 * the order they are written in, are exactly what they were.
 *
 * An interface, with a pass-through default, so the many unit tests that
 * build a repository by hand keep compiling and behaving exactly as before;
 * Hilt binds the real one ([of]), and the tests that pin the batching hand
 * it in themselves.
 */

package me.nettrash.familyconnect.data.db

import androidx.room.withTransaction

interface DbTransactions {

    /** Run [block] as one transaction: all of its writes commit together, or none do. */
    suspend fun <R> run(block: suspend () -> R): R

    companion object {

        /** No transaction: each write commits on its own, as it did before this seam existed. */
        val Direct: DbTransactions = object : DbTransactions {
            override suspend fun <R> run(block: suspend () -> R): R = block()
        }

        /** Room's own suspending transaction over [database]. */
        fun of(database: AppDatabase): DbTransactions = object : DbTransactions {
            override suspend fun <R> run(block: suspend () -> R): R = database.withTransaction(block)
        }
    }
}
