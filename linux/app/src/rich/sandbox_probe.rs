//! The sandbox preflight (shell-design §9.5, D22; md's `probe_sandbox`): WebKit's web processes
//! start inside bubblewrap, and Ubuntu 26.04 restricts unprivileged user namespaces to AppArmor
//! profiles that allow them — so where the app's profile (`/etc/apparmor.d/family-connect`) is
//! missing, bwrap cannot start and WebKit `g_error`s and ABORTS THE WHOLE APP (md facts §9).
//!
//! So before the first `WebView` exists the renderer runs `bwrap --unshare-user --ro-bind / / true`
//! once as a child (`gio::Subprocess`, awaited — about 10 ms). If it fails there is no rich renderer
//! for the session: blocks show their source, one banner says formulas and diagrams cannot be drawn
//! and names the profile — and, unlike md, THE APP KEEPS RUNNING: a chat client must not quit over
//! its diagrams.
//!
//! The app never sets `WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS`. When somebody else has (the Docker
//! dev image, `scripts/dev.sh`), WebKit starts no sandbox and there is nothing to probe.

/// The variable WebKit reads to start its web processes without bubblewrap.
pub const NO_SANDBOX_VARIABLE: &str = "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS";

/// What the preflight found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preflight {
    /// Not run, and why (WebKit will start no sandbox).
    Skipped(&'static str),
    /// bwrap made its user namespace: WebKit's will start too.
    Passed,
    /// bwrap could not (its exit status or why it could not be run — never more than that is
    /// logged).
    Failed(String),
}

impl Preflight {
    /// Whether a `WebView` may be made: anything but a failure.
    pub fn allows_webkit(&self) -> bool {
        !matches!(self, Preflight::Failed(_))
    }
}

/// Why the probe is not run, given whether the variable is set (non-empty).
pub fn skip_reason(sandbox_disabled: bool) -> Option<&'static str> {
    sandbox_disabled.then_some("WebKit's sandbox is disabled by its environment variable")
}

/// Whether this process's environment disables WebKit's sandbox.
pub fn sandbox_disabled() -> bool {
    std::env::var_os(NO_SANDBOX_VARIABLE).is_some_and(|value| !value.is_empty())
}

/// The argument vector the probe runs — as WebKit's web processes start.
pub const PROBE: [&str; 6] = ["bwrap", "--unshare-user", "--ro-bind", "/", "/", "true"];

/// Runs the preflight (see the module's comment).
pub async fn run() -> Preflight {
    if let Some(reason) = skip_reason(sandbox_disabled()) {
        return Preflight::Skipped(reason);
    }
    let argv: Vec<&std::ffi::OsStr> = PROBE.iter().map(std::ffi::OsStr::new).collect();
    let flags = gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
    let process = match gio::Subprocess::newv(&argv, flags) {
        Ok(process) => process,
        // No bwrap at all: WebKit could not start one either.
        Err(_) => return Preflight::Failed("bwrap could not be run".to_string()),
    };
    match process.wait_future().await {
        Ok(()) if process.is_successful() => Preflight::Passed,
        Ok(()) => Preflight::Failed(format!("bwrap exited with {}", process.exit_status())),
        Err(_) => Preflight::Failed("bwrap was not waited for".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_starts_bwrap_as_webkit_does() {
        assert_eq!(PROBE.join(" "), "bwrap --unshare-user --ro-bind / / true");
    }

    #[test]
    fn a_disabled_sandbox_is_not_probed_and_only_a_failure_stops_webkit() {
        assert!(skip_reason(true).is_some());
        assert_eq!(skip_reason(false), None);
        assert!(Preflight::Skipped("x").allows_webkit());
        assert!(Preflight::Passed.allows_webkit());
        assert!(!Preflight::Failed("no".into()).allows_webkit());
    }
}
