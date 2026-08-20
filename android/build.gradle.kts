// Root build file — Family Connect (Android)
//
// Nothing to build at the root; just declare the plugin versions (from
// gradle/libs.versions.toml) so the :app module can apply them.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.ksp) apply false
    alias(libs.plugins.hilt) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    // Declared here (classpath only) so :app can apply it *conditionally* —
    // the plugin hard-requires a google-services.json, which is user-supplied
    // and never committed. See the Firebase block in app/build.gradle.kts.
    alias(libs.plugins.google.services) apply false
}
