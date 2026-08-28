import java.util.Properties
import java.util.concurrent.atomic.AtomicBoolean

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
    alias(libs.plugins.kotlin.serialization)
}

// versionCode strategy: read from `version.properties` at the android/ root
// and auto-incremented after every successful assemble*/bundle* by the
// `bumpVersionCode` finalizer below. Mirrors the iOS app's `agvtool bump`
// post-build action — every Build button press bumps the number.
//
// The file IS tracked in git so the bump propagates between machines
// (commit it after a release, just like you'd commit the iOS pbxproj diff).
//
// versionName is human (semver). Defaults to "1.0" but can be overridden
// at the command line — the release workflow passes `-PversionName=1.2.3`
// derived from the v1.2.3 git tag, so a tag = a published version name.
val versionPropsFile = rootProject.file("version.properties")
val versionProps = Properties().apply {
    if (versionPropsFile.exists()) {
        versionPropsFile.inputStream().use(::load)
    }
}
val storedVersionCode: Int = run {
    val raw = versionProps.getProperty("versionCode")
    if (raw == null) {
        // No file present (e.g. fresh checkout that forgot to commit
        // version.properties, or a shallow CI clone). Fail loudly here
        // instead of defaulting to a small number that Play will already
        // have reserved from a previous upload.
        throw GradleException(
            "version.properties is missing or has no `versionCode` entry. " +
                "Either commit ${versionPropsFile.relativeTo(rootProject.projectDir)} " +
                "to the repo, or pass an explicit `-PversionCode=N` (where N is " +
                "strictly greater than every versionCode previously uploaded to Play)."
        )
    }
    val override = (project.findProperty("versionCode") as String?)?.toIntOrNull()
    override ?: raw.toInt()
}

val resolvedVersionName: String =
    (project.findProperty("versionName") as String?)?.takeIf { it.isNotBlank() } ?: "1.0"

// Allow opting out of the bump for a single build, e.g. when running a
// throwaway test or when CI does not want the local file mutated:
//     ./gradlew :app:assembleDebug -PnoBump
val skipVersionBump: Boolean = project.hasProperty("noBump")

// Resolve release signing material from (in order):
//   1. `keystore.properties` next to the root build file (developer machines).
//   2. FAMILYCONNECT_KEYSTORE_PATH / FAMILYCONNECT_KEYSTORE_PASSWORD /
//      FAMILYCONNECT_KEY_ALIAS / FAMILYCONNECT_KEY_PASSWORD env vars (CI).
// Returns `null` when nothing is configured — the release build then falls
// back to the debug signing config so `assembleRelease` still works locally
// without keys (it just won't be uploadable to Play).
// The Google Maps API key, for the map on a shared location.
//
// NEVER in the repo. Same shape as the signing config below: a gitignored
// `local.properties` at the android/ root (`MAPS_API_KEY=...`), or a
// `FAMILYCONNECT_MAPS_API_KEY` env var for CI. Absent is a supported state,
// not a broken build — without a key the bubble simply draws the pin card
// and never asks Google for anything, which is exactly what a fork of this
// repo with no key of its own should do. `BuildConfig.HAS_MAPS_KEY` is what
// the UI branches on, so an unkeyed build never shows Google's grey
// "authorization failure" tile.
val mapsApiKey: String = run {
    val propsFile = rootProject.file("local.properties")
    val fromFile = if (propsFile.exists()) {
        Properties().apply { propsFile.inputStream().use(::load) }.getProperty("MAPS_API_KEY")
    } else {
        null
    }
    fromFile ?: System.getenv("FAMILYCONNECT_MAPS_API_KEY") ?: ""
}

val releaseSigning: Map<String, String>? = run {
    val propsFile = rootProject.file("keystore.properties")
    if (propsFile.exists()) {
        val p = Properties().apply { propsFile.inputStream().use(::load) }
        mapOf(
            "storeFile" to (p.getProperty("storeFile") ?: return@run null),
            "storePassword" to (p.getProperty("storePassword") ?: return@run null),
            "keyAlias" to (p.getProperty("keyAlias") ?: return@run null),
            "keyPassword" to (p.getProperty("keyPassword") ?: return@run null),
        )
    } else {
        val path = System.getenv("FAMILYCONNECT_KEYSTORE_PATH")
        val storePassword = System.getenv("FAMILYCONNECT_KEYSTORE_PASSWORD")
        val keyAlias = System.getenv("FAMILYCONNECT_KEY_ALIAS")
        val keyPassword = System.getenv("FAMILYCONNECT_KEY_PASSWORD")
        if (path != null && storePassword != null && keyAlias != null && keyPassword != null) {
            mapOf(
                "storeFile" to path,
                "storePassword" to storePassword,
                "keyAlias" to keyAlias,
                "keyPassword" to keyPassword,
            )
        } else null
    }
}

android {
    namespace = "me.nettrash.familyconnect"
    compileSdk = 36
    // 36.1 (Android 16.1): what core-telecom 1.1.x — the Phone app's call
    // log for Family calls (TelecomCalls) — compiles against. targetSdk
    // stays 36; nothing in behaviour changes with the compile floor.
    compileSdkMinor = 1

    defaultConfig {
        applicationId = "me.nettrash.familyconnect"
        minSdk = 26
        targetSdk = 36
        versionCode = storedVersionCode
        versionName = resolvedVersionName

        // Read by the manifest's com.google.android.geo.API_KEY meta-data.
        // Empty is fine: the map is never drawn without HAS_MAPS_KEY, so an
        // unkeyed build shows the pin card and asks Google for nothing.
        manifestPlaceholders["mapsApiKey"] = mapsApiKey
        buildConfigField("boolean", "HAS_MAPS_KEY", mapsApiKey.isNotEmpty().toString())
    }

    // Predefined default server, as product flavors so the variant is
    // selectable (and sticky) in Android Studio's Build Variants panel —
    // the same lesson as the iOS FamilyConnect-nettrash scheme: a -P
    // property worked on the CLI but silently produced the generic build
    // on every IDE Run. `standard` keeps the ask-for-server first run for
    // source builds; `nettrash` ships pre-pointed at the hosted instance.
    // The task-name churn (assembleNettrashRelease, …) is the accepted
    // cost. Consumed via the DefaultServerUrl seam in di/AppModule.kt.
    flavorDimensions += "distribution"
    productFlavors {
        create("standard") {
            dimension = "distribution"
            buildConfigField("String", "DEFAULT_SERVER_URL", "\"\"")
        }
        create("nettrash") {
            dimension = "distribution"
            buildConfigField("String", "DEFAULT_SERVER_URL", "\"https://fc.nettrash.me\"")
        }
    }

    signingConfigs {
        releaseSigning?.let { sig ->
            create("release") {
                storeFile = rootProject.file(sig.getValue("storeFile"))
                storePassword = sig.getValue("storePassword")
                keyAlias = sig.getValue("keyAlias")
                keyPassword = sig.getValue("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            // If `keystore.properties` / env-vars aren't present, this stays
            // null and AGP falls back to the debug signing config so local
            // `assembleRelease` still works (just not uploadable to Play).
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        }
    }
    buildFeatures {
        compose = true
        // Needed for `BuildConfig.VERSION_NAME` references in Settings.
        // AGP 8+ generates BuildConfig only when this flag is on.
        buildConfig = true
    }
    testOptions {
        unitTests {
            // Robolectric needs Android resources + the system AndroidManifest
            // (Room DAO tests and Keystore-free settings tests run on it).
            isIncludeAndroidResources = true
            isReturnDefaultValues = true
        }
    }
    packaging {
        resources {
            excludes += setOf(
                "/META-INF/{AL2.0,LGPL2.1}",
                "/META-INF/DEPENDENCIES"
            )
        }
    }
}

// ---- Firebase / google-services (conditional) ----------------------------
//
// The repo deliberately ships WITHOUT a google-services.json — it is the
// user's own Firebase project config, added after cloning. The plugin
// hard-fails when the json is missing, so it is applied only when one is
// actually present. Recommended placement is FLAVOR-SCOPED:
//
//     app/src/nettrash/google-services.json
//
// so plain-source `standard` builds stay Firebase-free (no Google project
// needed to build and run against your own server), while the hosted
// `nettrash` flavor gets push. An app-level app/google-services.json also
// works and then covers every flavor.
//
// When the json exists for SOME flavors only, the missing ones must still
// assemble — strategy WARN instead of the default hard ERROR: that
// variant simply ships without Firebase config, FirebaseApp never
// initializes, and every Firebase call site no-ops at runtime (see
// data/push/PushTokenProvider.kt and FcPushService.kt).
val googleServicesJsonPresent = listOf(
    "google-services.json",           // app-level: covers all flavors
    "src/standard/google-services.json",
    "src/nettrash/google-services.json",
).any { file(it).exists() }
if (googleServicesJsonPresent) {
    apply(plugin = "com.google.gms.google-services")
    extensions.configure<com.google.gms.googleservices.GoogleServicesPlugin.GoogleServicesPluginConfig> {
        missingGoogleServicesStrategy =
            com.google.gms.googleservices.GoogleServicesPlugin.MissingGoogleServicesStrategy.WARN
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.core.telecom)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.ui)
    implementation(libs.androidx.ui.graphics)
    implementation(libs.androidx.ui.tooling.preview)
    implementation(libs.androidx.material3)
    implementation(libs.androidx.material.icons.extended)
    implementation(libs.androidx.navigation.compose)

    // Room — offline message store; the app renders from Room, never from
    // the wire directly (the socket is a live hint, REST is the truth).
    implementation(libs.androidx.room.runtime)
    implementation(libs.androidx.room.ktx)
    ksp(libs.androidx.room.compiler)

    // Hilt
    implementation(libs.hilt.android)
    ksp(libs.hilt.compiler)
    implementation(libs.androidx.hilt.navigation.compose)
    implementation(libs.androidx.hilt.lifecycle.viewmodel.compose)

    // Serialization & Network. OkHttp covers both REST and the WebSocket —
    // the author-sanctioned house stack (see exchange-android / Scan.Android);
    // Retrofit/Ktor would add a second HTTP abstraction for zero new ability.
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.okhttp)

    // DataStore — server URL, family status, and profile snapshot.
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.androidx.exifinterface)

    // Video transcoding for photo/video messages (docs/protocol.md,
    // "Photos and videos"). Only the sending side: a clip over the size
    // ceiling is re-encoded to 720p before upload. Playback uses the
    // platform's VideoView, so no ExoPlayer/media3-ui here.
    implementation(libs.androidx.media3.transformer)
    implementation(libs.androidx.media3.effect)

    // Firebase Cloud Messaging — push notifications (docs/protocol.md,
    // "Push notifications"). Messaging is the ONLY Firebase artifact: no
    // analytics, no crashlytics — the app's privacy posture is that user
    // data goes to the family's own server and nowhere else; FCM is the
    // unavoidable transport for waking a dead app. The dependency always
    // compiles in; without a google-services.json (see the conditional
    // plugin block above) FirebaseApp never initializes and all call
    // sites no-op at runtime.
    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)

    // WebRTC — peer-to-peer voice calls (docs/protocol.md, "Voice calls").
    // The audio travels device to device; the family's server only
    // relays the signalling frames over the socket it already holds.
    implementation(libs.stream.webrtc.android)

    // Maps — the map drawn on a shared location. See the note in
    // libs.versions.toml: this is the second Google service and a
    // deliberate exception nettrash asked for, not a drift. Drawn in LITE
    // MODE, behind a Settings switch, and only when an API key is
    // configured.
    implementation(libs.play.services.maps)
    implementation(libs.maps.compose)

    debugImplementation(libs.androidx.ui.tooling)

    // Unit tests — JUnit 4 + Truth; Robolectric where android.* types are
    // needed (in-memory Room, DataStore files); coroutines-test for the
    // virtual-clock backoff / ack-timeout / debounce tests.
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.truth)
    testImplementation(libs.kotlinx.coroutines.test)
    // Compose UI tests as local Robolectric tests (NoteDialogTest): the BOM
    // pins the versions. ui-test-manifest is debugImplementation — the
    // ComponentActivity createComposeRule launches must be in the VARIANT's
    // merged manifest (which Robolectric reads), not on the test classpath.
    testImplementation(platform(libs.androidx.compose.bom))
    testImplementation(libs.androidx.ui.test.junit4)
    debugImplementation(libs.androidx.ui.test.manifest)
}

// ---- IDE compatibility: legacy aggregate test-class tasks --------------
//
// AGP 9 stopped creating the legacy aggregate `unitTestClasses` /
// `androidTestClasses` tasks and only registers the variant-specific
// compile tasks (e.g. `compileDebugUnitTestKotlin`,
// `compileDebugAndroidTestKotlin`). Android Studio's "Make Project" /
// Gradle sync still invokes the aggregate names on some code paths and
// fails with "Cannot locate tasks that match ':app:<name>'".
// Register thin aliases so the IDE is happy. Pure aggregators — no actions
// of their own, just dependsOn the per-variant compile tasks.
afterEvaluate {
    if (tasks.findByName("unitTestClasses") == null) {
        tasks.register("unitTestClasses") {
            group = "verification"
            description =
                "Compatibility alias — depends on all variant-specific unit-test compile tasks."
            dependsOn(tasks.matching {
                val n = it.name
                n.startsWith("compile") &&
                    (n.endsWith("UnitTestKotlin") || n.endsWith("UnitTestJavaWithJavac"))
            })
        }
    }
    if (tasks.findByName("androidTestClasses") == null) {
        tasks.register("androidTestClasses") {
            group = "verification"
            description =
                "Compatibility alias — depends on all variant-specific instrumentation-test compile tasks."
            dependsOn(tasks.matching {
                val n = it.name
                n.startsWith("compile") &&
                    (n.endsWith("AndroidTestKotlin") || n.endsWith("AndroidTestJavaWithJavac"))
            })
        }
    }
}

// ---- versionCode auto-bump ----------------------------------------------
//
// Mirrors the iOS app's `agvtool bump` post-build action: every successful
// `assembleDebug`, `assembleRelease`, `bundleDebug`, or `bundleRelease`
// rewrites `version.properties` with `versionCode + 1`. The new value is
// effective on the *next* build (the current build keeps the value it was
// configured with, since defaultConfig is locked at configuration time).
//
// `doLast` only fires when the parent task's actions complete successfully,
// so failed builds don't bump. The AtomicBoolean guards against double-
// bumping when more than one of the listed tasks runs in a single
// invocation (e.g. `./gradlew assembleRelease bundleRelease`).
//
// Opt out per-build with `-PnoBump`.
val bumpedInThisInvocation = AtomicBoolean(false)
afterEvaluate {
    if (skipVersionBump) return@afterEvaluate
    // Aggregate tasks (assembleDebug) AND per-flavor ones
    // (assembleNettrashRelease) — running either must bump exactly once.
    listOf("assemble", "bundle").flatMap { prefix ->
        listOf("", "Standard", "Nettrash").flatMap { flavor ->
            listOf("Debug", "Release").map { type -> "$prefix$flavor$type" }
        }
    }.forEach { taskName ->
        tasks.findByName(taskName)?.doLast {
            if (!bumpedInThisInvocation.compareAndSet(false, true)) return@doLast
            val newValue = storedVersionCode + 1
            versionProps.setProperty("versionCode", newValue.toString())
            versionPropsFile.outputStream().use {
                versionProps.store(
                    it,
                    "Auto-incremented after build. Edit only if you know what you're doing."
                )
            }
            logger.lifecycle(
                ":app: bumped versionCode $storedVersionCode -> $newValue (effective next build)"
            )
        }
    }
}
