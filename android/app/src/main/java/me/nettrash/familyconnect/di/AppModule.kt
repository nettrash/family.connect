/*
 * AppModule.kt
 * Family Connect (Android)
 *
 * The single Hilt module. Everything app-scoped is wired here:
 *
 *   - Json          — ignoreUnknownKeys (protocol compatibility rule),
 *                     classDiscriminator "type" (WS frames),
 *                     encodeDefaults=false (never send fields the user
 *                     didn't set).
 *   - OkHttpClient  — ONE shared client: REST and the WebSocket share a
 *                     connection pool and dispatcher.
 *   - Room + DAOs   — no fallbackToDestructiveMigration, see AppDatabase.
 *   - DataStore     — preference state.
 *   - @AppScope     — SupervisorJob so one crashed collector doesn't
 *                     take down the socket manager and repositories.
 *
 * @Binds pairs each interface with its production impl; tests construct
 * fakes by hand — no Hilt in unit tests.
 *
 * iOS counterpart: ios/FamilyConnect/App/AppContainer.swift
 */

package me.nettrash.familyconnect.di

import android.content.ContentResolver
import android.content.Context
import me.nettrash.familyconnect.calls.AndroidCallAudio
import me.nettrash.familyconnect.calls.CallAudio
import me.nettrash.familyconnect.calls.CallManager
import me.nettrash.familyconnect.calls.CallMediaClient
import me.nettrash.familyconnect.calls.CallStarter
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.calls.CallTimings
import me.nettrash.familyconnect.calls.WebRtcClientFactory
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.preferencesDataStoreFile
import androidx.room.Room
import dagger.Binds
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.BuildConfig
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.ChatDao
import me.nettrash.familyconnect.data.db.LocalDataWiper
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.MessageDao
import me.nettrash.familyconnect.data.db.NoteDao
import me.nettrash.familyconnect.data.net.AndroidConnectivityObserver
import me.nettrash.familyconnect.data.net.ApiClient
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.AvatarApi
import me.nettrash.familyconnect.data.net.BoardApi
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.DefaultAuthApi
import me.nettrash.familyconnect.data.net.DefaultAttachmentApi
import me.nettrash.familyconnect.data.net.DefaultAvatarApi
import me.nettrash.familyconnect.data.net.DefaultBoardApi
import me.nettrash.familyconnect.data.net.DefaultChatApi
import me.nettrash.familyconnect.data.net.DefaultFamilyApi
import me.nettrash.familyconnect.data.net.FamilyApi
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.OkHttpChatSocket
import me.nettrash.familyconnect.data.repo.AvatarSource
import me.nettrash.familyconnect.data.repo.ContentResolverAvatarSource
import me.nettrash.familyconnect.data.repo.DefaultShareImporter
import me.nettrash.familyconnect.data.repo.ShareImporter
import me.nettrash.familyconnect.data.push.FirebasePushTokenProvider
import me.nettrash.familyconnect.data.push.PushTokenProvider
import me.nettrash.familyconnect.data.settings.DataStoreSettingsRepository
import me.nettrash.familyconnect.data.settings.DefaultServerUrl
import me.nettrash.familyconnect.data.settings.KeystoreTokenStore
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.TokenStore
import me.nettrash.familyconnect.util.Clock
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit
import javax.inject.Qualifier
import javax.inject.Singleton

/** Application-lifetime CoroutineScope (SupervisorJob + Default). */
@Qualifier
@Retention(AnnotationRetention.BINARY)
annotation class AppScope

/** ApiClient's "session is gone" broadcast, injectable without ApiClient. */
@Qualifier
@Retention(AnnotationRetention.BINARY)
annotation class UnauthorizedEvents

@Module
@InstallIn(SingletonComponent::class)
abstract class AppModule {

    @Binds
    abstract fun bindSettingsRepository(impl: DataStoreSettingsRepository): SettingsRepository

    @Binds
    abstract fun bindAuthApi(impl: DefaultAuthApi): AuthApi

    @Binds
    abstract fun bindAvatarApi(impl: DefaultAvatarApi): AvatarApi

    @Binds
    abstract fun bindAttachmentApi(impl: DefaultAttachmentApi): AttachmentApi

    @Binds
    abstract fun bindBoardApi(impl: DefaultBoardApi): BoardApi

    @Binds
    abstract fun bindAvatarSource(impl: ContentResolverAvatarSource): AvatarSource

    @Binds
    abstract fun bindFamilyApi(impl: DefaultFamilyApi): FamilyApi

    @Binds
    abstract fun bindChatApi(impl: DefaultChatApi): ChatApi

    @Binds
    abstract fun bindConnectivityObserver(impl: AndroidConnectivityObserver): ConnectivityObserver

    // The OS share target's copy-to-cache step. An interface so
    // MainViewModel's tests can default it away (see ShareImporter).
    @Binds
    abstract fun bindShareImporter(impl: DefaultShareImporter): ShareImporter

    @Binds
    abstract fun bindChatSocket(impl: OkHttpChatSocket): ChatSocket

    // Voice calls. The media seam is what keeps org.webrtc out of every
    // test; the audio seam does the same for AudioManager.
    @Binds
    abstract fun bindCallMediaFactory(impl: WebRtcClientFactory): CallMediaClient.Factory

    @Binds
    abstract fun bindCallAudio(impl: AndroidCallAudio): CallAudio

    @Binds
    abstract fun bindCallStarter(impl: CallManager): CallStarter

    @Binds
    abstract fun bindCallStateSource(impl: CallManager): CallStateSource

    // The only Firebase touchpoint in the graph. Safely inert when the
    // build has no google-services.json — see PushTokenProvider.kt.
    @Binds
    abstract fun bindPushTokenProvider(impl: FirebasePushTokenProvider): PushTokenProvider

    companion object {

        @Provides
        @Singleton
        fun provideJson(): Json = Json {
            // Protocol compatibility rule: clients MUST ignore unknown
            // fields and rely on the "type" discriminator for WS frames.
            ignoreUnknownKeys = true
            classDiscriminator = "type"
            encodeDefaults = false
        }

        @Provides
        @Singleton
        fun provideOkHttpClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .writeTimeout(30, TimeUnit.SECONDS)
            .build()

        @Provides
        @Singleton
        @AppScope
        fun provideAppScope(): CoroutineScope =
            CoroutineScope(SupervisorJob() + Dispatchers.Default)

        @Provides
        @Singleton
        fun provideDataStore(@ApplicationContext context: Context): DataStore<Preferences> =
            PreferenceDataStoreFactory.create {
                context.preferencesDataStoreFile("familyconnect.settings")
            }

        @Provides
        @Singleton
        fun provideDatabase(@ApplicationContext context: Context): AppDatabase =
            Room.databaseBuilder(context, AppDatabase::class.java, "familyconnect.db")
                // Deliberately NO fallbackToDestructiveMigration — this
                // is the family's message history. See AppDatabase.
                .addMigrations(
                    AppDatabase.MIGRATION_1_2,
                    AppDatabase.MIGRATION_2_3,
                    AppDatabase.MIGRATION_3_4,
                    AppDatabase.MIGRATION_4_5,
                    AppDatabase.MIGRATION_5_6,
                    AppDatabase.MIGRATION_6_7,
                    AppDatabase.MIGRATION_7_8,
                    AppDatabase.MIGRATION_8_9,
                    AppDatabase.MIGRATION_9_10,
                    AppDatabase.MIGRATION_10_11,
                    AppDatabase.MIGRATION_11_12,
                    AppDatabase.MIGRATION_12_13,
                    AppDatabase.MIGRATION_13_14,
                    AppDatabase.MIGRATION_14_15,
                    AppDatabase.MIGRATION_15_16,
                )
                .build()

        @Provides
        @Singleton
        fun provideContentResolver(@ApplicationContext context: Context): ContentResolver =
            context.contentResolver

        @Provides
        fun provideNoteDao(db: AppDatabase): NoteDao = db.noteDao()

        @Provides
        fun provideChatDao(db: AppDatabase): ChatDao = db.chatDao()

        @Provides
        fun provideMessageDao(db: AppDatabase): MessageDao = db.messageDao()

        @Provides
        fun provideMemberDao(db: AppDatabase): MemberDao = db.memberDao()

        @Provides
        @Singleton
        fun provideLocalDataWiper(db: AppDatabase): LocalDataWiper =
            LocalDataWiper { db.wipeAll() }

        @Provides
        @Singleton
        fun provideTokenStore(@ApplicationContext context: Context): TokenStore =
            KeystoreTokenStore(context)

        @Provides
        @Singleton
        @UnauthorizedEvents
        fun provideUnauthorizedEvents(apiClient: ApiClient): SharedFlow<Unit> =
            apiClient.unauthorized

        @Provides
        @Singleton
        fun provideClock(): Clock = Clock { System.currentTimeMillis() }

        /** The protocol's ring timeout and the client's own guards. */
        @Provides
        @Singleton
        fun provideCallTimings(): CallTimings = CallTimings()

        // The one place BuildConfig leaks into the object graph. Store
        // builds (the `nettrash` product flavor) compile the hosted instance in
        // (see app/build.gradle.kts); source builds leave it empty →
        // null → first run still asks for a server.
        @Provides
        @Singleton
        fun provideDefaultServerUrl(): DefaultServerUrl = DefaultServerUrl {
            BuildConfig.DEFAULT_SERVER_URL.takeIf { it.isNotBlank() }
        }
    }
}
