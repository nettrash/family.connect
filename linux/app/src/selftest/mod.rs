//! `family-connect --selftest OUT [--only g,…] [--no-input]` — the in-app self-test (shell-design
//! §27), compiled only with the cargo feature `selftest` (md's harness, md shell-design §13).
//!
//! The shell cannot be unit-tested without a display, so its proofs run inside the real app: a
//! private (`NON_UNIQUE`) process whose profile lives under `OUT/home/` (it refuses anything else:
//! [`check_home`]) opens its window as usual; this runner drives it group by group, records named
//! checks and writes `OUT/report.json`:
//!
//! ```json
//! {"started": "…", "finished": "…", "groups": ["shell", …],
//!  "checks": [{"name": "shell.app_starts", "passed": true, "detail": "…"}]}
//! ```
//!
//! The process exits 0 when every check passed inside the budget, else 1. Screenshots go to
//! `OUT/screenshots/<group>-<name>.png` (ImageMagick `import` of the Xvfb root window); real
//! keystrokes and clicks come from `xdotool` (XTEST). `scripts/dev.sh selftest` runs it under Xvfb
//! and a private session bus (`dbus-run-session`), with `gnome-keyring-daemon --unlock` for P5's
//! token round trip.
//!
//! **Adding a group** (each package its own, W.1 "Proof"): a file `selftest/<group>.rs` with
//! `pub async fn run(h: &Harness)`, its name in [`GROUPS`] and a line in `run_groups` — those two
//! lines are the only edit to this file. Checks go through [`Harness::check`]; the fake server is
//! [`Harness::fake_server`] (one per run, extended by the group's own routes); `--no-input` skips
//! the keystroke tier (ask [`Harness::input`] before typing).

pub mod attachments;
pub mod board;
pub mod calls;
pub mod chat;
pub mod desktop;
pub mod fake_server;
pub mod family;
pub mod media;
pub mod paid;
pub mod rich;
pub mod settings;
pub mod shell;

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;

use crate::shell::Shell;
use fake_server::FakeServer;

/// The variables that must point into `OUT/`.
pub const XDG_VARIABLES: [&str; 4] = [
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
];

/// Every group, in run order. Packages add theirs (`chat`, `attachments`, `rich`, `media`,
/// `board`, `family`, `settings`, `paid`, `desktop`, `calls`), `privacy` last. `desktop` runs right
/// after `shell`. GLib chooses its notification backend at the FIRST notification or withdrawal —
/// `shell`'s offline launch already opens a cached chat, which withdraws its notification — so the
/// desktop group's fake `org.gtk.Notifications` goes on the bus before the first group
/// ([`desktop::serve_notifications`]), as GNOME Shell's is from the start of a real session.
/// Without it GLib falls back to the freedesktop backend, whose numeric ids and `action::target`
/// strings the checks do not read.
pub const GROUPS: [&str; 11] = [
    "shell",
    "desktop",
    "chat",
    "attachments",
    "rich",
    "media",
    "board",
    "family",
    "settings",
    "paid",
    "calls",
];

/// The report's file name in `OUT`.
pub const REPORT_FILE_NAME: &str = "report.json";

/// The whole run's ceiling.
const BUDGET: Duration = Duration::from_secs(20 * 60);
/// How long the first window may take to appear.
const FIRST_WINDOW_LIMIT: Duration = Duration::from_secs(30);

const FLAG: &str = "--selftest";
const ONLY: &str = "--only";
const NO_INPUT: &str = "--no-input";

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The output folder: report, screenshots, the redirected profile under `home/`.
    pub out: PathBuf,
    /// The groups to run, in [`GROUPS`] order.
    pub groups: Vec<&'static str>,
    /// False with `--no-input`.
    pub input: bool,
}

impl Options {
    /// Takes `--selftest OUT [--only g,…] [--no-input]` out of `args`; `Ok(None)` without
    /// `--selftest`.
    pub fn take(args: &mut Vec<OsString>) -> Result<Option<Options>, String> {
        let Some(at) = args.iter().position(|a| a == FLAG) else {
            return Ok(None);
        };
        let usage = || {
            format!(
                "usage: family-connect {FLAG} OUT [{ONLY} group,…] [{NO_INPUT}]; groups: {}",
                GROUPS.join(", ")
            )
        };
        let out = match args.get(at + 1) {
            Some(out) if !out.to_string_lossy().starts_with('-') && !out.is_empty() => {
                PathBuf::from(out)
            }
            _ => return Err(usage()),
        };
        args.drain(at..at + 2);
        let mut input = true;
        if let Some(at) = args.iter().position(|a| a == NO_INPUT) {
            args.remove(at);
            input = false;
        }
        let mut groups: Vec<&'static str> = GROUPS.to_vec();
        let only_at = args
            .iter()
            .position(|a| a == ONLY || a.to_string_lossy().starts_with("--only="));
        if let Some(at) = only_at {
            let arg = args.remove(at).to_string_lossy().into_owned();
            let list = match arg.strip_prefix("--only=") {
                Some(list) => list.to_string(),
                None if at < args.len() => args.remove(at).to_string_lossy().into_owned(),
                None => return Err(usage()),
            };
            let mut wanted = Vec::new();
            for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                match GROUPS.iter().find(|g| **g == name) {
                    Some(group) => wanted.push(*group),
                    None => return Err(format!("unknown self-test group \"{name}\"; {}", usage())),
                }
            }
            if wanted.is_empty() {
                return Err(usage());
            }
            groups.retain(|g| wanted.contains(g));
        }
        Ok(Some(Options { out, groups, input }))
    }
}

/// A self-test writes settings, a cache and a session, and must never write them into a real
/// profile. GLib reads the XDG variables once, and `std::env::set_var` is `unsafe` in edition 2024
/// (forbidden here), so the process cannot redirect itself: `scripts/dev.sh selftest` exports them
/// under `OUT/home/`, and a run whose variables point anywhere else refuses to start (§2.3).
pub fn check_home(out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let given = std::path::absolute(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let real = std::fs::canonicalize(out).map_err(|e| format!("{}: {e}", out.display()))?;
    for variable in XDG_VARIABLES {
        let Some(value) = std::env::var_os(variable).filter(|v| !v.is_empty()) else {
            return Err(format!(
                "{variable} is not set; it must point into {}",
                given.display()
            ));
        };
        let path = Path::new(&value);
        if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            return Err(format!(
                "{variable}={} must be an absolute path without `..`",
                path.display()
            ));
        }
        if !path.starts_with(&given) && !path.starts_with(&real) {
            return Err(format!(
                "{variable}={} is not inside {}",
                path.display(),
                given.display()
            ));
        }
        std::fs::create_dir_all(path)
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        let resolved =
            std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !resolved.starts_with(&real) {
            return Err(format!(
                "{variable}={} resolves to {} outside it",
                path.display(),
                resolved.display()
            ));
        }
    }
    Ok(())
}

/// One row of the report.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Check {
    name: String,
    passed: bool,
    detail: String,
}

/// What every group drives the app with: checks, waits, screenshots, input, the fake server.
pub struct Harness {
    out: PathBuf,
    input: bool,
    group: Cell<&'static str>,
    checks: RefCell<Vec<Check>>,
    fake: RefCell<Option<Rc<FakeServer>>>,
}

impl Harness {
    fn new(options: &Options) -> Harness {
        Harness {
            out: options.out.clone(),
            input: options.input,
            group: Cell::new(""),
            checks: RefCell::new(Vec::new()),
            fake: RefCell::new(None),
        }
    }

    fn record(&self, name: String, passed: bool, detail: String) -> bool {
        println!(
            "CHECK {} {name}{}",
            if passed { "pass" } else { "FAIL" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {detail}")
            }
        );
        self.checks.borrow_mut().push(Check {
            name,
            passed,
            detail,
        });
        passed
    }

    fn all_passed(&self) -> bool {
        self.checks.borrow().iter().all(|c| c.passed)
    }

    /// The group being run.
    pub fn group(&self) -> &'static str {
        self.group.get()
    }

    /// Records `<group>.<name>`; returns `passed` so a group can stop early.
    pub fn check(&self, name: &str, passed: bool, detail: impl Into<String>) -> bool {
        let name = match self.group() {
            "" => name.to_string(),
            group => format!("{group}.{name}"),
        };
        self.record(name, passed, detail.into())
    }

    pub async fn sleep(&self, duration: Duration) {
        glib::timeout_future(duration).await;
    }

    /// Polls `condition` every 50 ms; true as soon as it holds, false after `limit`.
    pub async fn wait_until(&self, limit: Duration, condition: impl Fn() -> bool) -> bool {
        let start = std::time::Instant::now();
        loop {
            if condition() {
                return true;
            }
            if start.elapsed() >= limit {
                return false;
            }
            glib::timeout_future(Duration::from_millis(50)).await;
        }
    }

    /// `OUT/screenshots/<group>-<name>.png` of the whole Xvfb screen, after a moment for the frame.
    pub async fn screenshot(&self, name: &str) -> Option<PathBuf> {
        self.sleep(Duration::from_millis(400)).await;
        let folder = self.out.join("screenshots");
        if let Err(err) = std::fs::create_dir_all(&folder) {
            eprintln!("selftest: cannot create {}: {err}", folder.display());
            return None;
        }
        let file = match self.group() {
            "" => format!("{name}.png"),
            group => format!("{group}-{name}.png"),
        };
        let path = folder.join(file);
        let (ok, output) = self
            .run(&["import", "-window", "root", &path.to_string_lossy()])
            .await;
        if ok {
            println!("SHOT {}", path.display());
            Some(path)
        } else {
            eprintln!("selftest: screenshot {name} failed: {output}");
            None
        }
    }

    /// Runs a program to completion without blocking the main loop: (success, stdout + stderr).
    pub async fn run(&self, argv: &[&str]) -> (bool, String) {
        let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        let flags = gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE;
        let process = match gio::Subprocess::newv(&argv, flags) {
            Ok(process) => process,
            Err(err) => return (false, err.to_string()),
        };
        match process.communicate_utf8_future(None).await {
            Ok((out, _)) => (
                process.is_successful(),
                out.map(|s| s.to_string()).unwrap_or_default(),
            ),
            Err(err) => (false, err.to_string()),
        }
    }

    fn report_json(&self, started: &str, finished: &str, groups: &[&str]) -> String {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"started\": {},", json_string(started));
        let _ = writeln!(out, "  \"finished\": {},", json_string(finished));
        let groups: Vec<String> = groups.iter().map(|g| json_string(g)).collect();
        let _ = writeln!(out, "  \"groups\": [{}],", groups.join(", "));
        let checks: Vec<String> = self
            .checks
            .borrow()
            .iter()
            .map(|c| {
                format!(
                    "    {{\"name\": {}, \"passed\": {}, \"detail\": {}}}",
                    json_string(&c.name),
                    c.passed,
                    json_string(&c.detail)
                )
            })
            .collect();
        if checks.is_empty() {
            out.push_str("  \"checks\": []\n");
        } else {
            let _ = writeln!(out, "  \"checks\": [\n{}\n  ]", checks.join(",\n"));
        }
        out.push_str("}\n");
        out
    }
}

/// The group-facing half: what P1–P6's groups drive the app with.
impl Harness {
    pub fn shell(&self) -> Rc<Shell> {
        Shell::get()
    }

    /// The run's fake server — started on first use, one per run, shared by every group (each adds
    /// its routes and its people).
    pub fn fake_server(&self) -> Rc<FakeServer> {
        if let Some(fake) = self.fake.borrow().as_ref() {
            return Rc::clone(fake);
        }
        let fake = FakeServer::start().expect("the fake server starts on loopback");
        *self.fake.borrow_mut() = Some(Rc::clone(&fake));
        fake
    }

    /// The output folder (`OUT`).
    pub fn out(&self) -> &Path {
        &self.out
    }

    /// The redirected profile, `OUT/home`.
    pub fn home(&self) -> PathBuf {
        self.out.join("home")
    }

    /// False with `--no-input`.
    pub fn input(&self) -> bool {
        self.input
    }

    /// `future`, or `None` after `limit`.
    pub async fn with_timeout<T>(
        &self,
        limit: Duration,
        future: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        glib::future_with_timeout(limit, future).await.ok()
    }

    pub async fn xdotool(&self, args: &[&str]) -> (bool, String) {
        let mut argv = vec!["xdotool"];
        argv.extend_from_slice(args);
        self.run(&argv).await
    }

    /// Presses a chord in xdotool's spelling (`ctrl+r`, `ctrl+question`). False with `--no-input`.
    pub async fn key(&self, chord: &str) -> bool {
        if !self.input {
            return false;
        }
        let (ok, _) = self.xdotool(&["key", "--clearmodifiers", chord]).await;
        self.sleep(Duration::from_millis(120)).await;
        ok
    }

    /// Types `text` key by key. False with `--no-input`.
    pub async fn type_text(&self, text: &str) -> bool {
        if !self.input {
            return false;
        }
        let (ok, _) = self.xdotool(&["type", "--delay", "40", "--", text]).await;
        self.sleep(Duration::from_millis(80)).await;
        ok
    }

    /// The X11 window id of `window`, found by its title among THIS process's visible windows
    /// (`--all --pid`: a second instance's window or a tooltip of the same name is never taken),
    /// the largest when several match (a popover's or a tooltip's surface is smaller).
    pub async fn xid(&self, window: &gtk::Window) -> Option<String> {
        let title = window.title().map(|t| t.to_string()).unwrap_or_default();
        let pattern = format!("^{}$", regex_escape(&title));
        let pid = std::process::id().to_string();
        let (_, ids) = self
            .xdotool(&[
                "search",
                "--all",
                "--onlyvisible",
                "--pid",
                &pid,
                "--name",
                &pattern,
            ])
            .await;
        let mut best: Option<(i64, String)> = None;
        for id in ids.lines().map(str::trim).filter(|id| !id.is_empty()) {
            let (_, geometry) = self.xdotool(&["getwindowgeometry", "--shell", id]).await;
            let field = |name: &str| {
                geometry
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .unwrap_or(0)
            };
            let area = field("WIDTH=") * field("HEIGHT=");
            if best.as_ref().is_none_or(|(most, _)| area >= *most) {
                best = Some((area, id.to_string()));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Gives `window` the keyboard focus. False with `--no-input` or when it cannot be found.
    pub async fn focus(&self, window: &gtk::Window) -> bool {
        if !self.input {
            return false;
        }
        let Some(xid) = self.xid(window).await else {
            return false;
        };
        let (ok, _) = self.xdotool(&["windowfocus", "--sync", &xid]).await;
        self.sleep(Duration::from_millis(150)).await;
        ok
    }

    /// Resizes `window` on the X server (Xvfb has no window manager to do it). False with
    /// `--no-input`.
    pub async fn resize(&self, window: &gtk::Window, width: i32, height: i32) -> bool {
        if !self.input {
            return false;
        }
        let Some(xid) = self.xid(window).await else {
            return false;
        };
        let (ok, _) = self
            .xdotool(&[
                "windowsize",
                "--sync",
                &xid,
                &width.to_string(),
                &height.to_string(),
            ])
            .await;
        self.sleep(Duration::from_millis(300)).await;
        ok
    }

    /// Clicks button 1 at (`x`, `y`) in `widget`'s coordinates. False with `--no-input`.
    pub async fn click(
        &self,
        window: &gtk::Window,
        widget: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) -> bool {
        if !self.input {
            return false;
        }
        let Some((sx, sy)) = self.screen_point(window, widget, x, y).await else {
            return false;
        };
        let (ok, _) = self
            .xdotool(&[
                "mousemove",
                "--sync",
                &sx.to_string(),
                &sy.to_string(),
                "click",
                "1",
            ])
            .await;
        self.sleep(Duration::from_millis(300)).await;
        ok
    }

    /// Clicks the middle of `widget`.
    pub async fn click_middle(&self, window: &gtk::Window, widget: &impl IsA<gtk::Widget>) -> bool {
        let (w, h) = (f64::from(widget.width()), f64::from(widget.height()));
        self.click(window, widget, w / 2.0, h / 2.0).await
    }

    /// Screen coordinates of a point in `widget`: the X11 window's origin, the surface transform
    /// and the widget's offset, scaled to device pixels (`GDK_SCALE`) — for the drags and the
    /// pointer moves the harness has no helper for.
    pub async fn screen_point(
        &self,
        window: &gtk::Window,
        widget: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
    ) -> Option<(i32, i32)> {
        let xid = self.xid(window).await?;
        let (_, geometry) = self.xdotool(&["getwindowgeometry", "--shell", &xid]).await;
        let field = |name: &str| {
            geometry
                .lines()
                .find_map(|l| l.strip_prefix(name))
                .and_then(|v| v.parse::<f64>().ok())
        };
        let (wx, wy) = (field("X=")?, field("Y=")?);
        let point = widget.compute_point(window, &gtk::graphene::Point::new(x as f32, y as f32))?;
        let (sx, sy) = window.surface_transform();
        let scale = window.surface().map_or(1.0, |surface| surface.scale());
        let device = |origin: f64, offset: f64| (origin + (offset * scale)).round() as i32;
        Some((
            device(wx, sx + f64::from(point.x())),
            device(wy, sy + f64::from(point.y())),
        ))
    }
}

/// `s` as a POSIX extended regular expression matching itself (xdotool's `--name`).
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A JSON string literal (RFC 8259: quote, backslash and every control character escaped).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn now_iso8601() -> String {
    glib::DateTime::now_utc()
        .and_then(|t| t.format_iso8601())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

thread_local! {
    /// The run's verdict, once the report is written.
    static OUTCOME: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Called in `startup`: holds the application and spawns the runner, which waits for the main
/// window, runs the groups, writes the report and quits.
pub fn start(shell: &Rc<Shell>, options: Options) {
    let hold = shell.app.hold();
    glib::spawn_future_local(async move {
        if let Err(err) = std::fs::create_dir_all(&options.out) {
            eprintln!("selftest: cannot create {}: {err}", options.out.display());
        }
        let harness = Rc::new(Harness::new(&options));
        let started = now_iso8601();
        println!(
            "selftest: {} → {}",
            options.groups.join(", "),
            options.out.display()
        );
        let completed = glib::future_with_timeout(
            BUDGET,
            run_groups(Rc::clone(&harness), options.groups.clone()),
        )
        .await
        .is_ok();
        if !completed {
            harness.record(
                "selftest.completed".to_string(),
                false,
                format!(
                    "the run exceeded its {} min budget in group \"{}\"",
                    BUDGET.as_secs() / 60,
                    harness.group()
                ),
            );
        }
        let finished = now_iso8601();
        let report = options.out.join(REPORT_FILE_NAME);
        let written = std::fs::write(
            &report,
            harness.report_json(&started, &finished, &options.groups),
        );
        let passed = match written {
            Ok(()) => {
                let checks = harness.checks.borrow();
                let failed = checks.iter().filter(|c| !c.passed).count();
                println!(
                    "selftest: {} checks, {failed} failed → {}",
                    checks.len(),
                    report.display()
                );
                completed && harness.all_passed()
            }
            Err(err) => {
                eprintln!("selftest: cannot write {}: {err}", report.display());
                false
            }
        };
        OUTCOME.with(|o| o.set(Some(passed)));
        drop(hold);
        // The real way out (§3.8): the connection disposed, the windows gone, then `app.quit()`.
        Shell::get().quit();
    });
}

async fn run_groups(harness: Rc<Harness>, groups: Vec<&'static str>) {
    let shell = Shell::get();
    let visible = harness
        .wait_until(FIRST_WINDOW_LIMIT, || {
            shell
                .main_window()
                .is_some_and(|window| window.window().is_visible())
        })
        .await;
    if !visible {
        eprintln!(
            "selftest: no window appeared within {} s",
            FIRST_WINDOW_LIMIT.as_secs()
        );
    }
    harness.screenshot("start").await;
    if groups.contains(&"desktop") {
        desktop::serve_notifications().await;
    }
    for group in groups {
        harness.group.set(group);
        println!("selftest: group {group}");
        let h: &Harness = &harness;
        match group {
            "shell" => shell::run(h).await,
            "attachments" => attachments::run(h).await,
            "rich" => rich::run(h).await,
            "media" => media::run(h).await,
            "board" => board::run(h).await,
            "family" => family::run(h).await,
            "settings" => settings::run(h).await,
            "paid" => paid::run(h).await,
            "desktop" => desktop::run(h).await,
            "chat" => chat::run(h).await,
            "calls" => calls::run(h).await,
            other => {
                harness.check("unknown", false, format!("no such group: {other}"));
            }
        }
    }
}

/// The process's exit code: success only when the report was written and every check passed.
pub fn exit_code() -> glib::ExitCode {
    match OUTCOME.with(Cell::get) {
        Some(true) => glib::ExitCode::SUCCESS,
        _ => glib::ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_flags_are_taken_and_the_rest_left() {
        let mut a = args(&[
            "fc",
            "--selftest",
            "/out/s",
            "--only",
            "shell",
            "--no-input",
            "x.png",
        ]);
        let options = Options::take(&mut a).unwrap().unwrap();
        assert_eq!(options.out, PathBuf::from("/out/s"));
        assert_eq!(options.groups, vec!["shell"]);
        assert!(!options.input);
        assert_eq!(a, args(&["fc", "x.png"]));
        let mut a = args(&["fc", "--only=shell", "--selftest", "out"]);
        assert_eq!(
            Options::take(&mut a).unwrap().unwrap().groups,
            vec!["shell"]
        );
        let mut none = args(&["fc", "--only", "shell"]);
        assert_eq!(Options::take(&mut none), Ok(None));
    }

    #[test]
    fn bad_command_lines_are_refused() {
        for bad in [
            &["fc", "--selftest"][..],
            &["fc", "--selftest", "--no-input"],
            &["fc", "--selftest", "out", "--only"],
            &["fc", "--selftest", "out", "--only", "shell,nope"],
            &["fc", "--selftest", "out", "--only="],
        ] {
            assert!(Options::take(&mut args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn json_strings_escape_what_rfc_8259_requires() {
        assert_eq!(
            json_string("a\"b\\c\nd\u{1}é"),
            "\"a\\\"b\\\\c\\nd\\u0001é\""
        );
    }

    #[test]
    fn titles_become_literal_patterns() {
        assert_eq!(regex_escape("(3) Family Connect"), "\\(3\\) Family Connect");
    }
}
