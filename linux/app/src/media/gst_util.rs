//! The small GStreamer chores every media piece shares: building a pipeline from a description,
//! running a one-shot pipeline to its end on the main context, the discoverer as a future, a
//! temporary file of the app's own, and an error as the log may say it (its GLib domain and code —
//! never its message, which can quote a path or a device's name: §26).
//!
//! Everything here runs on the GLib main context. GStreamer's own threads only post to buses and
//! fire `Send` callbacks that hop back through a `futures_channel` (§2.4) — never an `Rc`.

use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fc_core::rt::Cancel;
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use gst::prelude::*;

/// Whether the plugin registry holds `factory` (an encoder, a sink).
pub fn has_element(factory: &str) -> bool {
    gst::ElementFactory::find(factory).is_some()
}

/// A GLib error as the log may say it: `"gst-resource-error-quark 3"`.
pub fn error_line(error: &glib::Error) -> String {
    format!("{} {}", error.domain().as_str(), error.code())
}

/// A failure out of a pipeline, as fc-logic's `AttemptFailure` and the log want it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineError {
    /// The GLib error domain (`gst-stream-error-quark`), or the shell's own word for what failed
    /// (`state-change`, `timeout`).
    pub domain: String,
    pub code: i32,
    /// The factory of the element that posted it, when known (never its name, which can carry a
    /// device's).
    pub factory: Option<String>,
}

impl PipelineError {
    pub fn new(domain: &str, code: i32) -> Self {
        PipelineError {
            domain: domain.to_string(),
            code,
            factory: None,
        }
    }

    /// From a bus `Error` message.
    pub fn from_message(error: &gst::message::Error) -> Self {
        let glib_error = error.error();
        PipelineError {
            domain: glib_error.domain().as_str().to_string(),
            code: glib_error.code(),
            factory: error
                .src()
                .and_then(|source| source.downcast_ref::<gst::Element>().cloned())
                .and_then(|element| element.factory())
                .map(|factory| factory.name().to_string()),
        }
    }

    /// fc-logic's form: the domain as the kind, the code as the number.
    pub fn attempt_failure(&self) -> fc_logic::transcode_attempts::AttemptFailure {
        fc_logic::transcode_attempts::AttemptFailure::new(
            self.domain.clone(),
            u32::try_from(self.code).unwrap_or(0),
        )
    }
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.domain, self.code)?;
        if let Some(factory) = &self.factory {
            write!(f, " from {factory}")?;
        }
        Ok(())
    }
}

/// A description launched as a pipeline. `Err` names only that it could not be parsed (a missing
/// element is the common case — the caller tries the next way).
pub fn launch(description: &str) -> Result<gst::Pipeline, PipelineError> {
    let element = gst::parse::launch(description).map_err(|error| PipelineError {
        domain: error.domain().as_str().to_string(),
        code: error.code(),
        factory: None,
    })?;
    match element.downcast::<gst::Pipeline>() {
        Ok(pipeline) => Ok(pipeline),
        // A description of one element parses to that element: put it in a pipeline.
        Err(element) => {
            let pipeline = gst::Pipeline::new();
            pipeline
                .add(&element)
                .map_err(|_| PipelineError::new("pipeline", 0))?;
            Ok(pipeline)
        }
    }
}

/// The named element of a pipeline or bin.
pub fn named(bin: &impl IsA<gst::Bin>, name: &str) -> Option<gst::Element> {
    bin.by_name(name)
}

/// Sets `location` on the named file element (`filesink name=out`, `filesrc name=src`).
pub fn set_location(bin: &impl IsA<gst::Bin>, name: &str, path: &Path) -> bool {
    match named(bin, name) {
        Some(element) => {
            element.set_property("location", path.to_string_lossy().as_ref());
            true
        }
        None => false,
    }
}

/// How a one-shot pipeline ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The bus said EOS.
    Eos,
    /// The cancel was raised (the pipeline is set to `Null` by the caller).
    Cancelled,
    /// It ran past `limit`.
    TimedOut,
}

/// Plays `pipeline` until its bus says EOS, an error, the cancel or `limit` — then leaves it in
/// whatever state it reached (the caller sets `Null`). `Err` for a refused state change or a bus
/// error.
pub async fn run_to_end(
    pipeline: &gst::Pipeline,
    cancel: &Cancel,
    limit: Duration,
) -> Result<Ended, PipelineError> {
    let Some(bus) = pipeline.bus() else {
        return Err(PipelineError::new("bus", 0));
    };
    let mut messages = bus.stream();
    if pipeline.set_state(gst::State::Playing).is_err() {
        return Err(first_error(&bus).unwrap_or_else(|| PipelineError::new("state-change", 0)));
    }
    let work = async {
        while let Some(message) = messages.next().await {
            match message.view() {
                gst::MessageView::Eos(_) => return Ok(Ended::Eos),
                gst::MessageView::Error(error) => return Err(PipelineError::from_message(error)),
                _ => {}
            }
        }
        Err(PipelineError::new("bus", 1))
    };
    let stop = async {
        let cancelled = cancel.cancelled();
        let deadline = glib::timeout_future(limit);
        match select(Box::pin(cancelled), Box::pin(deadline)).await {
            Either::Left(_) => Ended::Cancelled,
            Either::Right(_) => Ended::TimedOut,
        }
    };
    match select(Box::pin(work), Box::pin(stop)).await {
        Either::Left((ended, _)) => ended,
        Either::Right((ended, _)) => Ok(ended),
    }
}

/// Waits until `pipeline` has prerolled (an `AsyncDone` after a change to `Paused` or a seek), an
/// error, or `limit`.
pub async fn until_async_done(
    pipeline: &gst::Pipeline,
    messages: &mut gst::bus::BusStream,
    limit: Duration,
) -> Result<(), PipelineError> {
    let work = async {
        while let Some(message) = messages.next().await {
            match message.view() {
                gst::MessageView::AsyncDone(_) => return Ok(()),
                gst::MessageView::Error(error) => return Err(PipelineError::from_message(error)),
                gst::MessageView::Eos(_) => return Err(PipelineError::new("eos", 0)),
                _ => {}
            }
        }
        Err(PipelineError::new("bus", 1))
    };
    let _ = pipeline;
    match select(Box::pin(work), Box::pin(glib::timeout_future(limit))).await {
        Either::Left((done, _)) => done,
        Either::Right(_) => Err(PipelineError::new("timeout", 0)),
    }
}

/// The first error already on `bus` (after a refused state change), if one is there.
pub fn first_error(bus: &gst::Bus) -> Option<PipelineError> {
    while let Some(message) = bus.pop() {
        if let gst::MessageView::Error(error) = message.view() {
            return Some(PipelineError::from_message(error));
        }
    }
    None
}

/// What the discoverer reads of `path` within `limit` (§10.7: 10 s), on the main context; `None`
/// for an error, a timeout, nothing readable.
pub async fn discover(path: &Path, limit: Duration) -> Option<gst_pbutils::DiscovererInfo> {
    let timeout =
        gst::ClockTime::from_nseconds(u64::try_from(limit.as_nanos()).unwrap_or(u64::MAX));
    let discoverer = gst_pbutils::Discoverer::new(timeout).ok()?;
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    discoverer.connect_discovered(move |_, info, error| {
        let answer = match error {
            Some(_) => None,
            None => Some(info.clone()),
        };
        if let Ok(mut slot) = sender.lock()
            && let Some(sender) = slot.take()
        {
            let _ = sender.send(answer);
        }
    });
    discoverer.start();
    if discoverer
        .discover_uri_async(&super::pipelines::file_uri(path))
        .is_err()
    {
        discoverer.stop();
        return None;
    }
    let answer = receiver.await.ok().flatten();
    discoverer.stop();
    let info = answer?;
    match info.result() {
        gst_pbutils::DiscovererResult::Ok => Some(info),
        _ => None,
    }
}

/// A new temporary file of the app's own under `folder`: `<prefix>-<uuid>.<extension>`, made now,
/// empty, mode `0600` (a recording is somebody's voice), its folder `0700`.
pub fn temp_file(folder: &Path, prefix: &str, extension: &str) -> std::io::Result<PathBuf> {
    crate::services::files::private_dir(folder)?;
    let path = folder.join(format!(
        "{prefix}-{}.{extension}",
        fc_core::protocol::send_pipeline::uuid_v4()
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    Ok(path)
}

/// Deletes a temporary; a failure is logged by its kind and forgotten.
pub fn remove_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => crate::fc_warn!("media: a temporary could not be removed ({})", error.kind()),
    }
}

/// A duration as GStreamer's clock time.
pub fn clock_time(seconds: f64) -> gst::ClockTime {
    if seconds.is_finite() && seconds > 0.0 {
        gst::ClockTime::from_nseconds((seconds * 1e9).min(u64::MAX as f64) as u64)
    } else {
        gst::ClockTime::ZERO
    }
}

/// A clock time in seconds (0 for none).
pub fn seconds(time: Option<gst::ClockTime>) -> f64 {
    time.map_or(0.0, |time| time.nseconds() as f64 / 1e9)
}
