/*
 * MigrationCoverageTest.kt
 * Family Connect (Android)
 *
 * The one test that would have caught v21 before a phone did.
 *
 * Every other test in this package proves a migration's SQL is right.
 * None of them proved Room is ever GIVEN it — and that is the failure
 * that shipped: MIGRATION_20_21 existed, was correct, and was not in the
 * list, so Room found no path out of 19, refused to open the database and
 * took the process down on launch ("A migration from 19 to 21 was
 * required but not found"). fallbackToDestructiveMigration is forbidden
 * here, so there is no soft landing: an unregistered migration is a crash
 * loop for anyone upgrading.
 *
 * The version is read from a database Room itself creates, not from the
 * @Database annotation — that one is CLASS-retention and invisible at
 * runtime — so what this compares the chain against is the number Room
 * will actually stamp and actually demand on the next upgrade. A version
 * bump with nothing registered fails here rather than in a user's hand.
 */

package me.nettrash.familyconnect.data.db

import androidx.room.Room
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class MigrationCoverageTest {

    /** What Room stamps on a fresh database — @Database(version) itself. */
    private val declaredVersion: Int
        get() {
            val fresh = Room.inMemoryDatabaseBuilder(
                RuntimeEnvironment.getApplication(),
                AppDatabase::class.java,
            ).build()
            try {
                return fresh.openHelper.writableDatabase.version
            } finally {
                fresh.close()
            }
        }

    @Test
    fun everyStepFromOneToTheDeclaredVersionIsRegistered() {
        val steps = AppDatabase.ALL_MIGRATIONS
            .associateBy { it.startVersion }
            .mapValues { (_, migration) -> migration.endVersion }

        val missing = (1 until declaredVersion).filter { steps[it] != it + 1 }

        assertThat(missing).isEmpty()
    }

    @Test
    fun theChainStopsExactlyAtTheDeclaredVersion() {
        // A migration past the @Database version means the annotation was
        // not bumped — the new column exists in the entity, the upgrade
        // never runs, and Room validates a schema nobody wrote.
        val highest = AppDatabase.ALL_MIGRATIONS.maxOf { it.endVersion }

        assertThat(highest).isEqualTo(declaredVersion)
    }

    @Test
    fun noVersionIsMigratedTwice() {
        // Two migrations out of the same version is Room picking one of
        // them by an order this test refuses to depend on.
        val starts = AppDatabase.ALL_MIGRATIONS.map { it.startVersion }

        assertThat(starts).containsNoDuplicates()
    }
}
