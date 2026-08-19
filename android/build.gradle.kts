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
}
