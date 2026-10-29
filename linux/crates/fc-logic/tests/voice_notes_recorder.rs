//! The voice recorder's seam, with fakes — no C# counterpart: on Windows this glue is the window's
//! own (`ChatsView.StartRecordingAsync`, `KeepAwake`), which the C# suite never ran. Held here:
//! the start notes itself for its whole length and is ended however it ends (the future dropped
//! included); the screen reader's "Recording" and its second come after the microphone opened and
//! before anything is recorded; an interruption during the start, or a window that no longer wants
//! the recording, lets go of the microphone it is granted; the encoding asked for is the protocol's
//! voice note; and the screen-on hold is balanced.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Clock, Event, LocalBoxFuture};
use fc_core::testing::FakeClock;
use fc_logic::media_encoding::{self, AacEncoding};
use fc_logic::voice_look::VoiceMeter;
use fc_logic::voice_notes::{
    self, CapabilityAccess, Inhibitor, KeepAwake, Recorded, RecordingFailure, RecordingStart,
    Started, VoiceRecorder, VoiceRecording,
};
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use time::macros::datetime;

#[derive(Default)]
struct FakeRecording {
    abandoned: Cell<bool>,
    failed: Event<()>,
}

impl VoiceMeter for FakeRecording {
    fn take_peak_dbfs(&self) -> Option<f64> {
        None
    }
}

impl VoiceRecording for FakeRecording {
    fn elapsed(&self) -> Duration {
        Duration::from_secs(3)
    }

    fn failed(&self) -> &Event<()> {
        &self.failed
    }

    fn stop(&self) -> LocalBoxFuture<'static, Recorded> {
        Box::pin(std::future::ready(Recorded {
            bytes: Some(vec![0; 2048]),
            elapsed: Duration::from_secs(3),
        }))
    }

    fn abandon(&self) {
        self.abandoned.set(true);
    }
}

/// A microphone that opens when `opened` is opened, runs the lead-in it is given, and then answers
/// — the recording, or `refusing`.
struct FakeRecorder {
    opened: common::Gate,
    refusing: Option<RecordingFailure>,
    asked: RefCell<Vec<Vec<AacEncoding>>>,
    recording: Rc<FakeRecording>,
    /// What happened, in order: "open", "speak", "record".
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl FakeRecorder {
    fn new(log: &Rc<RefCell<Vec<&'static str>>>) -> Self {
        let opened = common::Gate::new();
        opened.open();
        FakeRecorder {
            opened,
            refusing: None,
            asked: RefCell::default(),
            recording: Rc::default(),
            log: Rc::clone(log),
        }
    }
}

impl VoiceRecorder for FakeRecorder {
    fn start(
        &self,
        encodings: Vec<AacEncoding>,
        speak_first: Option<LocalBoxFuture<'static, ()>>,
    ) -> LocalBoxFuture<'static, Result<Rc<dyn VoiceRecording>, RecordingFailure>> {
        self.asked.borrow_mut().push(encodings);
        let opened = self.opened.wait();
        let refusing = self.refusing;
        let recording: Rc<dyn VoiceRecording> = self.recording.clone();
        let log = Rc::clone(&self.log);
        Box::pin(async move {
            opened.await;
            if let Some(failure) = refusing {
                return Err(failure);
            }
            log.borrow_mut().push("open");
            if let Some(speak) = speak_first {
                speak.await;
            }
            log.borrow_mut().push("record");
            Ok(recording)
        })
    }

    fn microphone_access(&self) -> CapabilityAccess {
        CapabilityAccess::Allowed
    }
}

fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-10-05 12:00:00 UTC))
}

type Outcome = Rc<RefCell<Option<Started>>>;

/// Spawn a start over `recorder`, its outcome landing in the answer.
fn spawn_start(
    pool: &LocalPool,
    recorder: &Rc<FakeRecorder>,
    start: &Rc<RecordingStart>,
    screen_reader: bool,
    clock: &FakeClock,
    log: &Rc<RefCell<Vec<&'static str>>>,
    wanted: Rc<Cell<bool>>,
) -> Outcome {
    let outcome = Outcome::default();
    let recorder = Rc::clone(recorder);
    let start = Rc::clone(start);
    let clock: Rc<dyn Clock> = Rc::new(clock.clone());
    let speak: Rc<dyn Fn()> = {
        let log = Rc::clone(log);
        Rc::new(move || log.borrow_mut().push("speak"))
    };
    let answer = Rc::clone(&outcome);
    pool.spawner()
        .spawn_local(async move {
            let started = voice_notes::start_recording(
                recorder.as_ref(),
                &start,
                screen_reader,
                clock,
                speak,
                move || wanted.get(),
            )
            .await;
            *answer.borrow_mut() = Some(started);
        })
        .unwrap();
    outcome
}

#[test]
fn a_start_asks_for_the_voice_note_and_ends_when_the_recorder_runs() {
    let log = Rc::default();
    let recorder = Rc::new(FakeRecorder::new(&log));
    let start = Rc::new(RecordingStart::new());
    let mut pool = LocalPool::new();
    let outcome = spawn_start(
        &pool,
        &recorder,
        &start,
        false,
        &clock(),
        &log,
        Rc::new(Cell::new(true)),
    );
    pool.run_until_stalled();

    match outcome.borrow_mut().take() {
        Some(Started::Recording { spoken, .. }) => assert!(!spoken),
        _ => panic!("the recorder runs"),
    }
    assert!(!start.starting());
    assert_eq!(vec!["open", "record"], *log.borrow());
    // The protocol's voice note: AAC-LC mono 64 000 — one encoding on Linux.
    assert_eq!(
        vec![media_encoding::voice_note_attempts()],
        *recorder.asked.borrow()
    );
    assert_eq!(1, recorder.asked.borrow()[0].len());
    assert_eq!(64_000, recorder.asked.borrow()[0][0].bitrate);
    assert_eq!(1, recorder.asked.borrow()[0][0].channels);
}

/// S6: with a screen reader running, "Recording" is said once the microphone has opened and a
/// second passes before anything is recorded; the start is under way — and playback refused — the
/// whole time.
#[test]
fn a_screen_reader_hears_recording_a_second_before_the_microphone_records() {
    let log = Rc::default();
    let recorder = Rc::new(FakeRecorder::new(&log));
    let start = Rc::new(RecordingStart::new());
    let clock = clock();
    let mut pool = LocalPool::new();
    let outcome = spawn_start(
        &pool,
        &recorder,
        &start,
        true,
        &clock,
        &log,
        Rc::new(Cell::new(true)),
    );
    pool.run_until_stalled();

    assert_eq!(vec!["open", "speak"], *log.borrow());
    assert!(start.starting());
    assert!(start.quiet(false));
    assert!(outcome.borrow().is_none());
    clock.advance(Duration::from_millis(999));
    pool.run_until_stalled();
    assert!(outcome.borrow().is_none());
    clock.advance(Duration::from_millis(1));
    pool.run_until_stalled();

    assert_eq!(vec!["open", "speak", "record"], *log.borrow());
    match outcome.borrow_mut().take() {
        Some(Started::Recording { spoken, .. }) => assert!(spoken),
        _ => panic!("the recorder runs"),
    }
    assert_eq!(vec![Duration::from_secs(1)], clock.slept());
    assert!(!start.starting());
}

/// S4: an interruption heard while the microphone opens finds no recorder to stop — the start lets
/// go of the microphone once it is granted, and records nothing.
#[test]
fn an_interruption_while_the_microphone_opens_lets_it_go() {
    let log = Rc::default();
    let mut slow = FakeRecorder::new(&log);
    slow.opened = common::Gate::new();
    let gate = slow.opened.clone();
    let recorder = Rc::new(slow);
    let start = Rc::new(RecordingStart::new());
    let mut pool = LocalPool::new();
    let outcome = spawn_start(
        &pool,
        &recorder,
        &start,
        false,
        &clock(),
        &log,
        Rc::new(Cell::new(true)),
    );
    pool.run_until_stalled();
    assert!(start.starting());

    start.interrupted();
    gate.open();
    pool.run_until_stalled();

    assert!(matches!(outcome.borrow().as_ref(), Some(Started::LetGo)));
    assert!(recorder.recording.abandoned.get());
    assert!(!start.starting());
    assert!(!start.was_interrupted());
}

/// The window's own reasons — the chat left, a call begun meanwhile — let go of the microphone the
/// same way.
#[test]
fn a_window_that_no_longer_wants_it_lets_it_go() {
    let log = Rc::default();
    let recorder = Rc::new(FakeRecorder::new(&log));
    let start = Rc::new(RecordingStart::new());
    let mut pool = LocalPool::new();
    let outcome = spawn_start(
        &pool,
        &recorder,
        &start,
        false,
        &clock(),
        &log,
        Rc::new(Cell::new(false)),
    );
    pool.run_until_stalled();

    assert!(matches!(outcome.borrow().as_ref(), Some(Started::LetGo)));
    assert!(recorder.recording.abandoned.get());
    assert!(!start.starting());
}

/// A microphone that would not open says why, and the start is over.
#[test]
fn a_refused_start_says_why() {
    let log = Rc::default();
    let mut refusing = FakeRecorder::new(&log);
    refusing.refusing = Some(RecordingFailure::CouldNotStart);
    let recorder = Rc::new(refusing);
    let start = Rc::new(RecordingStart::new());
    let mut pool = LocalPool::new();
    let outcome = spawn_start(
        &pool,
        &recorder,
        &start,
        true,
        &clock(),
        &log,
        Rc::new(Cell::new(true)),
    );
    pool.run_until_stalled();

    assert!(matches!(
        outcome.borrow().as_ref(),
        Some(Started::Failed(RecordingFailure::CouldNotStart))
    ));
    assert!(log.borrow().is_empty());
    assert!(!start.starting());
}

/// A start whose future is dropped — the window gone — is over: the next press may start again.
#[test]
fn a_dropped_start_is_over() {
    let log = Rc::default();
    let mut slow = FakeRecorder::new(&log);
    slow.opened = common::Gate::new();
    let recorder = Rc::new(slow);
    let start = Rc::new(RecordingStart::new());
    let clock: Rc<dyn Clock> = Rc::new(clock());
    {
        let mut future = Box::pin(voice_notes::start_recording(
            recorder.as_ref(),
            &start,
            false,
            clock,
            Rc::new(|| {}),
            || true,
        ));
        let waker = futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert!(future.as_mut().poll(&mut context).is_pending());
        assert!(start.starting());
    }
    assert!(!start.starting());
}

#[derive(Default)]
struct FakeInhibitor {
    next: Cell<u32>,
    granting: Cell<bool>,
    given_back: RefCell<Vec<u32>>,
}

impl Inhibitor for FakeInhibitor {
    fn inhibit(&self) -> u32 {
        if !self.granting.get() {
            return 0;
        }
        self.next.set(self.next.get() + 1);
        self.next.get()
    }

    fn uninhibit(&self, cookie: u32) {
        self.given_back.borrow_mut().push(cookie);
    }
}

/// BALANCED (S1.7): a second hold is the same hold, a release with nothing held does nothing, and a
/// request the desktop could not grant is not held — the next hold asks again.
#[test]
fn the_screen_is_kept_on_once_and_given_back_once() {
    let inhibitor = Rc::new(FakeInhibitor::default());
    let awake = KeepAwake::new(inhibitor.clone());
    awake.release();
    assert!(inhibitor.given_back.borrow().is_empty());

    assert!(!awake.hold());
    assert!(!awake.is_held());
    inhibitor.granting.set(true);
    assert!(awake.hold());
    assert!(awake.hold());
    assert_eq!(1, inhibitor.next.get());
    awake.release();
    awake.release();
    assert_eq!(vec![1], *inhibitor.given_back.borrow());

    // Dropped while held, it gives the request back.
    assert!(awake.hold());
    drop(awake);
    assert_eq!(vec![1, 2], *inhibitor.given_back.borrow());
}
