//! How the app says which build it is, on the one line in Settings that says so
//! (`AppVersionText.cs`).
//!
//! The line reads the way the Apple apps' does — "1.3 (437)", the version people talk about and
//! then the build in brackets — and drops the brackets entirely at build 0, which is what a local
//! build carries (CI is what stamps a number). A bare "1.3 (0)" would be a bug report quoting a
//! number that means "nobody stamped this". (On Windows the build is the THIRD part of an MSIX
//! version because the Store reserves the fourth; printed inline, "1.1.437" would advertise a
//! patch release that does not exist. Settings says "Family Connect for Linux 1.3 (build)",
//! shell-design §13.)

/// The version line's value: "1.1 (437)", or "1.1" for an unstamped build. Digits are ASCII
/// whatever the reader's locale: Rust's integer formatting has no culture.
pub fn for_version(major: u32, minor: u32, build: u32) -> String {
    if build > 0 {
        format!("{major}.{minor} ({build})")
    } else {
        format!("{major}.{minor}")
    }
}
