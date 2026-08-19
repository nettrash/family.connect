// settings.gradle.kts — Family Connect (Android)
//
// Repository policy mirrors Scan.Android: plugins resolve from Google's
// maven (scoped by group regex so unrelated lookups don't hit it),
// dependencies resolve centrally with FAIL_ON_PROJECT_REPOS so no module
// can quietly add its own repository. The foojay resolver provisions a
// JDK toolchain when the machine's default JVM doesn't match.
pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}
plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "0.10.0"
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "FamilyConnect"
include(":app")
