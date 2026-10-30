//! The `media` group (P3, shell-design §27, W.1 "Proof"): GStreamer's test sources stand in for the
//! microphone and the camera (`media::use_test_sources` — a `selftest` build only), the fake server
//! serves the attachments, and a minimal `ComposerHost` stands in for P1's composer.
//!
//! Proved here: a video file plays and pauses; two players never play at once; a voice note in a
//! bubble is fetched through the attachment cache and plays; a voice note records from the test
//! tone, its meter moves, the screen is held on, it is sent through the composer protocol as an M4A
//! (`moov` first) with its waveform; a recording is parked SYNCHRONOUSLY at a simulated quit and
//! restored from disk; an interruption stops and keeps; a circle plays inside its circle (the
//! corners clipped — a pixel test) and stops when scrolled away; the video recorder opens on the
//! window's overlay with the test camera, records, reviews and sends a 480 × 480 square; a picked
//! 1080p60 video is transcoded to the profile (read back by the discoverer and by
//! `gst-discoverer-1.0`) and a cancel stages nothing and leaves nothing; an animation's raw encode
//! starts `ftyp` and has its `moov` after `mdat` (no faststart).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto, ServerUrl};
use fc_core::rt::{Cancel, Clock};
use fc_core::store::{KeptTranscript, TranscriptStore};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::animated_pictures::animation_prep;
use fc_logic::animation_preparing::{self, FrameFeed, H264Profile};
use fc_logic::composer_button::Recording;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::not_sent::RecordingEnd;
use fc_logic::parked_recordings::ParkedRecordings;
use fc_logic::round_look::Playing;
use fc_logic::round_playback::RoundPhase;
use fc_logic::round_recorder::RecorderStage;
use fc_logic::transcode_attempts::Attempted;
use fc_logic::transcript_sound::{MadeSound, SoundAttempt, SoundTrackMaker};
use fc_logic::{animation_timing, faststart, media_encoding};

use super::Harness;
use super::fake_server::FakeResponse;
use crate::media;
use crate::media::preparing::Picked;
use crate::seams::composer::ComposerHost;
use crate::seams::conversation::ConversationSlots;
use crate::seams::media::{MediaSource, Player, PlayerState};
use crate::windows::main_window::GatePage;

const WAIT: Duration = Duration::from_secs(20);
/// The chat the test composer writes to.
const CHAT: i64 = 4242;
/// The attachments the fake serves.
const VOICE_ID: i64 = 9001;
const CIRCLE_ID: i64 = 9002;

/// A minimal `ComposerHost` (P1's composer is not in this worktree).
struct TestHost {
    window: gtk::Window,
    staged: RefCell<Vec<StagedMedia>>,
    sent: RefCell<Vec<StagedMedia>>,
    changes: Cell<u32>,
}

impl ComposerHost for TestHost {
    fn chat_id(&self) -> i64 {
        CHAT
    }

    fn draft(&self) -> String {
        String::new()
    }

    fn staged(&self) -> usize {
        self.staged.borrow().len()
    }

    fn window(&self) -> Option<gtk::Window> {
        Some(self.window.clone())
    }

    fn stage(&self, note: StagedMedia) {
        self.staged.borrow_mut().push(note);
    }

    fn send(&self, media: StagedMedia) {
        self.sent.borrow_mut().push(media);
    }

    fn changed(&self) {
        self.changes.set(self.changes.get() + 1);
    }
}

pub async fn run(h: &Harness) {
    media::use_test_sources(true);
    let shell = h.shell();
    let folder = h.out().join("media");
    let _ = std::fs::create_dir_all(&folder);

    // ---- test files, made by GStreamer itself ------------------------------------------------------
    let clip = folder.join("clip.mp4");
    let voice = folder.join("voice.m4a");
    let square = folder.join("square.mp4");
    let big = folder.join("big.mp4");
    let long = folder.join("long.mp4");
    let made = make(h, &[
        &format!("videotestsrc num-buffers=90 pattern=smpte ! video/x-raw,width=640,height=360,framerate=30/1 ! x264enc ! video/x-h264,profile=high ! h264parse ! mp4mux name=m faststart=true ! filesink location={} audiotestsrc num-buffers=130 ! audio/x-raw,rate=44100,channels=1 ! avenc_aac ! aacparse ! m.", clip.display()),
        &format!("audiotestsrc num-buffers=130 wave=sine freq=330 ! audio/x-raw,rate=44100,channels=1 ! avenc_aac bitrate=64000 ! aacparse ! mp4mux faststart=true ! filesink location={}", voice.display()),
        &format!("videotestsrc num-buffers=90 pattern=smpte ! video/x-raw,width=480,height=480,framerate=30/1 ! x264enc ! video/x-h264,profile=high ! h264parse ! mp4mux name=m faststart=true ! filesink location={} audiotestsrc num-buffers=130 ! audio/x-raw,rate=44100,channels=1 ! avenc_aac ! aacparse ! m.", square.display()),
        &format!("videotestsrc num-buffers=120 pattern=snow ! video/x-raw,width=1920,height=1080,framerate=60/1 ! x264enc speed-preset=ultrafast bitrate=16000 ! video/x-h264,profile=high ! h264parse ! mp4mux name=m ! filesink location={} audiotestsrc num-buffers=94 ! audio/x-raw,rate=48000,channels=2 ! avenc_aac ! aacparse ! m.", big.display()),
        &format!("videotestsrc num-buffers=900 pattern=smpte ! video/x-raw,width=1920,height=1080,framerate=60/1 ! x264enc speed-preset=ultrafast ! video/x-h264,profile=high ! h264parse ! mp4mux ! filesink location={}", long.display()),
    ]).await;
    if !h.check("test_files_made", made, folder.display().to_string()) {
        return;
    }

    // ---- a circle's sound, made for its text (media::sound_tracks) --------------------------------
    {
        let tracks = media::sound_tracks::GstSoundTracks::new(folder.clone());
        let held: Rc<[u8]> = std::fs::read(&square).unwrap_or_default().into();
        let reading = tracks.read(&held, "mp4").await;
        let copied = tracks
            .make(&held, "mp4", &SoundAttempt::PASSTHROUGH, Cancel::new())
            .await;
        let encode = SoundAttempt::reencode(media_encoding::voice_note_attempts()[0]);
        let encoded = tracks.make(&held, "mp4", &encode, Cancel::new()).await;
        let sound_only = |made: &Result<MadeSound, _>| {
            made.as_ref().is_ok_and(|made| {
                made.codec.as_deref() == Some("aac")
                    && !made.has_video
                    && made.bytes.get(4..8) == Some(b"ftyp".as_slice())
            })
        };
        h.check(
            "circle_sound_for_its_text",
            matches!(&reading, Ok(Some(track)) if track.codec == "aac")
                && sound_only(&copied)
                && sound_only(&encoded),
            format!(
                "read {:?}; copied {:?}; encoded {:?}",
                reading
                    .as_ref()
                    .map(|r| r.as_ref().map(|t| t.codec.clone())),
                copied
                    .as_ref()
                    .map(|m| (m.codec.clone(), m.has_video, m.bytes.len())),
                encoded
                    .as_ref()
                    .map(|m| (m.codec.clone(), m.has_video, m.bytes.len())),
            ),
        );
    }

    // ---- the fake server, a session --------------------------------------------------------------------
    let fake = h.fake_server();
    let voice_bytes = Rc::new(std::fs::read(&voice).unwrap_or_default());
    let square_bytes = Rc::new(std::fs::read(&square).unwrap_or_default());
    let poster = Rc::new(
        media::preparing::poster(&square, Some((480, 480)), 480)
            .await
            .unwrap_or_default(),
    );
    {
        let (voice_bytes, square_bytes, poster) =
            (voice_bytes.clone(), square_bytes.clone(), poster.clone());
        fake.route(
            "GET",
            "/api/v1/attachments/*",
            Rc::new(move |_, request| {
                let body = match request.segments.get(3).map(String::as_str) {
                    Some(id) if id == VOICE_ID.to_string() => voice_bytes.to_vec(),
                    Some(id) if id == CIRCLE_ID.to_string() => square_bytes.to_vec(),
                    _ => return FakeResponse::error(404, "attachment_not_found"),
                };
                FakeResponse { status: 200, body }
            }),
        );
        fake.route(
            "GET",
            "/api/v1/attachments/*/preview",
            Rc::new(
                move |_, request| match request.segments.get(3).map(String::as_str) {
                    Some(id) if id == CIRCLE_ID.to_string() => FakeResponse {
                        status: 200,
                        body: poster.to_vec(),
                    },
                    _ => FakeResponse::error(404, "attachment_not_found"),
                },
            ),
        );
    }
    let mia = fake.add_user("media-mia", "password", "Mia");
    let _family = fake.add_family("Media Testers", mia);
    let server = ServerUrl::normalise(Some(&fake.url())).expect("the fake's address");
    let same_server = shell
        .connection()
        .is_some_and(|connection| connection.server == server);
    if !same_server {
        shell.open_server(server.clone());
        h.wait_until(WAIT, || {
            shell.connection().is_some() && !shell.gate_facts().booting
        })
        .await;
    }
    let Some(connection) = shell.connection() else {
        h.check("session", false, "no connection to the fake server");
        return;
    };
    let signed = connection.session.sign_in("media-mia", "password").await;
    let main = shell.main_window();
    let chats = h
        .wait_until(WAIT, || {
            main.as_ref()
                .is_some_and(|main| main.page() == GatePage::Chats)
        })
        .await;
    h.check(
        "session",
        matches!(signed, Ok(None)) && chats,
        format!("{:?}", signed.map(|e| e.map(|e| e.code))),
    );

    // ---- a window of our own for what is drawn ---------------------------------------------------------
    let window = gtk::Window::builder()
        .title("Media test")
        .default_width(620)
        .default_height(560)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_margin_top(12);
    column.set_margin_start(12);
    column.set_margin_end(12);
    window.set_child(Some(&column));
    window.present();
    h.sleep(Duration::from_millis(500)).await;

    // ---- the player --------------------------------------------------------------------------------------
    let player = shell.players.get().create(true);
    let picture = gtk::Picture::builder()
        .width_request(320)
        .height_request(180)
        .content_fit(gtk::ContentFit::Contain)
        .build();
    picture.set_paintable(player.paintable().as_ref());
    column.append(&picture);
    player.load(MediaSource::File(clip.clone()));
    player.play();
    let played = h
        .wait_until(WAIT, || {
            player.state() == PlayerState::Playing && player.position() > 0.5
        })
        .await;
    h.check(
        "player_plays",
        played && player.paintable().is_some(),
        format!(
            "{:?} at {:.2} s of {:.2}",
            player.state(),
            player.position(),
            player.duration()
        ),
    );
    h.screenshot("player").await;
    player.pause();
    let paused = h
        .wait_until(WAIT, || player.state() == PlayerState::Paused)
        .await;
    let at = player.position();
    h.sleep(Duration::from_millis(600)).await;
    h.check(
        "player_pauses",
        paused && (player.position() - at).abs() < 0.05,
        format!("{:?}, {:.2} → {:.2}", player.state(), at, player.position()),
    );

    // ---- an authorised stream: the bearer on every request (§1.4) -----------------------------------------
    let token = fc_core::protocol::TokenStore::token(connection.tokens.as_ref());
    let streamed = shell.players.get().create(true);
    let before = fake
        .requests()
        .iter()
        .filter(|r| *r == &format!("GET /api/v1/attachments/{CIRCLE_ID}"))
        .count();
    streamed.load(MediaSource::Stream {
        url: format!("{}/api/v1/attachments/{CIRCLE_ID}", fake.url()),
        bearer: token.clone(),
    });
    streamed.play();
    let flowing = h
        .wait_until(WAIT, || {
            streamed.state() == PlayerState::Playing && streamed.position() > 0.3
        })
        .await;
    let asked = fake
        .requests()
        .iter()
        .filter(|r| *r == &format!("GET /api/v1/attachments/{CIRCLE_ID}"))
        .count()
        - before;
    h.check(
        "player_streams_with_the_bearer",
        token.is_some() && flowing && asked >= 1,
        format!(
            "{:?} at {:.2} s, {asked} request(s)",
            streamed.state(),
            streamed.position()
        ),
    );
    streamed.stop();
    // Without the token the fake answers 401: the player fails, it never plays.
    let refused = shell.players.get().create(false);
    refused.load(MediaSource::Stream {
        url: format!("{}/api/v1/attachments/{CIRCLE_ID}", fake.url()),
        bearer: None,
    });
    refused.play();
    let failed = h
        .wait_until(WAIT, || refused.state() == PlayerState::Failed)
        .await;
    h.check(
        "player_stream_without_the_bearer_fails",
        failed,
        format!("{:?}", refused.state()),
    );
    refused.stop();

    // ---- one thing at a time -----------------------------------------------------------------------------
    let registry = shell.now_playing.get();
    let first = shell.players.get().create(false);
    let second = shell.players.get().create(false);
    first.load(MediaSource::File(voice.clone()));
    second.load(MediaSource::File(voice.clone()));
    let claim = |player: &Rc<dyn Player>| {
        let weak = Rc::downgrade(player);
        registry.claim(
            Playing::VoiceNote,
            Rc::new(move || {
                if let Some(player) = weak.upgrade() {
                    player.pause();
                }
            }),
        )
    };
    claim(&first);
    first.play();
    h.wait_until(WAIT, || first.state() == PlayerState::Playing)
        .await;
    claim(&second);
    second.play();
    let one = h
        .wait_until(WAIT, || {
            second.state() == PlayerState::Playing && first.state() == PlayerState::Paused
        })
        .await;
    h.check(
        "one_thing_plays",
        one,
        format!("first {:?}, second {:?}", first.state(), second.state()),
    );
    // An interruption pauses what S4 says: a call pauses a voice note.
    registry.interrupt(fc_logic::round_look::PlaybackEvent::Call);
    let interrupted = h
        .wait_until(WAIT, || second.state() == PlayerState::Paused)
        .await;
    h.check(
        "a_call_pauses_a_voice_note",
        interrupted,
        format!("{:?}", second.state()),
    );
    first.stop();
    second.stop();
    player.stop();
    column.remove(&picture);

    // ---- the voice bubble ----------------------------------------------------------------------------------
    let message = MessageDto {
        id: 77,
        chat_id: CHAT,
        sender_id: mia + 1000,
        ..MessageDto::default()
    };
    let note = AttachmentDto {
        id: VOICE_ID,
        kind: "audio".to_string(),
        mime: Some("audio/mp4".to_string()),
        duration_ms: Some(3_000),
        waveform: Some("0123456789abcdeffedcba98765432100123456789abcdef".to_string()),
        ..AttachmentDto::default()
    };
    let (bubble, bubble_widget) = media::voice_bubble::VoiceBubble::new(&message, &note, false);
    column.append(&bubble_widget);
    h.sleep(Duration::from_millis(300)).await;
    h.check(
        "voice_bubble_final_size",
        bubble_widget.width_request() == fc_logic::voice_look::WIDTH as i32,
        format!("{}", bubble_widget.width_request()),
    );
    bubble.press_play();
    let fetched_and_playing = h
        .wait_until(WAIT, || bubble.player_state() == Some(PlayerState::Playing))
        .await;
    h.check(
        "voice_bubble_plays_through_the_cache",
        fetched_and_playing
            && fake
                .requests()
                .iter()
                .any(|r| r == &format!("GET /api/v1/attachments/{VOICE_ID}")),
        format!("{:?}", bubble.player_state()),
    );
    h.sleep(Duration::from_millis(800)).await;
    h.screenshot("voice-bubble").await;
    bubble.press_play();
    column.remove(&bubble_widget);
    drop(bubble);

    // ---- the text under a voice note (media::transcript) ---------------------------------------------------
    // Text this device holds is always offered, and showing it asks nobody: kept here, then shown
    // and folded away through the real slot.
    let said = "Bring the tent, the long one.";
    let kept = TranscriptStore::new(&connection.cache).keep(
        &KeptTranscript {
            attachment_id: VOICE_ID,
            text: said.to_string(),
            language: Some("en".to_string()),
            supplied: false,
        },
        shell.clock.now(),
    );
    let with_text = media::MediaSlots.voice(&message, &note, false);
    column.append(&with_text);
    h.sleep(Duration::from_millis(300)).await;
    let say = shell.say.clone();
    let offered = super::settings::button(&with_text, say.get("Show text"));
    if let Some(show) = &offered {
        show.emit_clicked();
    }
    let shown = h
        .wait_until(WAIT, || {
            super::settings::find::<gtk::Label>(&with_text, |label| label.label() == said).is_some()
        })
        .await;
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("voice-text").await;
    if let Some(hide) = super::settings::button(&with_text, say.get("Hide text")) {
        hide.emit_clicked();
    }
    let folded = h
        .wait_until(WAIT, || {
            super::settings::button(&with_text, say.get("Show text")).is_some()
        })
        .await;
    h.check(
        "voice_text_shown_and_folded",
        kept.is_ok() && offered.is_some() && shown && folded,
        format!(
            "kept {}, offered {}, shown {shown}, folded {folded}",
            kept.is_ok(),
            offered.is_some()
        ),
    );
    column.remove(&with_text);

    // ---- a voice note through the composer protocol ----------------------------------------------------------
    let host = Rc::new(TestHost {
        window: window.clone(),
        staged: RefCell::new(Vec::new()),
        sent: RefCell::new(Vec::new()),
        changes: Cell::new(0),
    });
    let recording = shell.composer_slots.get().attach(host.clone());
    h.check(
        "can_record_with_the_test_tone",
        recording.can_record() && recording.recording() == Recording::None,
        format!("{:?}", recording.recording()),
    );
    let row_holder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.append(&row_holder);
    row_holder.append(&recording.row());
    recording.start_voice(false);
    let started = h
        .wait_until(WAIT, || recording.recording() == Recording::HandsFree)
        .await;
    h.check(
        "voice_recording_starts",
        started,
        format!("{:?}", recording.recording()),
    );
    let composer = media::recording::installed()
        .and_then(|slots| slots.composers().into_iter().find(|c| c.chat_id() == CHAT));
    // P5's one keep-awake (`desktop::keep_awake::hold`, Awake::VoiceMessage): asked for the
    // recording's life — granted or not (no session manager answers in a container).
    h.check(
        "the_screen_is_held_on_while_recording",
        composer.as_ref().is_some_and(|c| c.keeps_awake()),
        format!("{:?}", composer.as_ref().map(|c| c.keeps_awake())),
    );
    h.sleep(Duration::from_millis(1800)).await;
    let heard = composer.as_ref().map_or(0, |c| c.meter_heard());
    h.check(
        "voice_meter_moves",
        heard > 5,
        format!("{heard} meter ticks"),
    );
    h.screenshot("recording-row").await;
    recording.send();
    let sent = h.wait_until(WAIT, || !host.sent.borrow().is_empty()).await;
    let note_sent = host.sent.borrow().first().cloned();
    let detail = note_sent.as_ref().map_or_else(
        || "nothing sent".to_string(),
        |note| {
            format!(
                "{} {} {} bytes, {:?} ms, waveform {:?}, moov first {:?}",
                note.kind,
                note.mime,
                note.bytes.len(),
                note.duration_ms,
                note.waveform.as_ref().map(String::len),
                faststart::is_moov_first(&note.bytes)
            )
        },
    );
    h.check(
        "voice_note_sent_through_the_composer",
        sent && note_sent.as_ref().is_some_and(|note| {
            note.kind == "audio"
                && note.mime == "audio/mp4"
                && note.name.is_none()
                && note.duration_ms.is_some_and(|ms| ms >= 1_000)
                && note.waveform.as_ref().is_some_and(|w| w.len() == 48)
                && note.bytes.get(4..8) == Some(b"ftyp".as_slice())
                && faststart::is_moov_first(&note.bytes) == Some(true)
        }),
        detail,
    );
    h.check(
        "the_screen_may_sleep_after",
        composer.as_ref().is_some_and(|c| !c.keeps_awake()),
        String::new(),
    );

    // ---- parked at a simulated quit, restored from disk ------------------------------------------------------
    recording.start_voice(false);
    h.wait_until(WAIT, || recording.recording() == Recording::HandsFree)
        .await;
    h.sleep(Duration::from_millis(1500)).await;
    if let Some(slots) = media::recording::installed() {
        // What `Shell::on_quitting` runs: synchronous, nothing awaited.
        slots.park_now();
    }
    let me = connection.session.state().me.map(|me| me.id).unwrap_or(0);
    let on_disk = ParkedRecordings::for_account(&shell.paths.parked_dir(), &server.to_string(), me);
    let parked = on_disk.of(CHAT);
    h.check(
        "parked_synchronously_at_quit",
        recording.recording() == Recording::None
            && parked.len() == 1
            && parked[0].duration_ms >= 1_000
            && on_disk
                .staged(&parked[0])
                .is_some_and(|note| faststart::is_moov_first(&note.bytes) == Some(true)),
        format!(
            "{} parked, {:?}",
            parked.len(),
            parked.first().map(|p| p.duration_ms)
        ),
    );
    // A composer attached after a restart finds it on disk: the not-sent row, and the slot dimmed.
    let again = shell.composer_slots.get().attach(host.clone());
    let rows = again.not_sent_rows();
    h.check(
        "restored_as_not_sent",
        again.not_sent() && rows.is_some(),
        format!("not sent {}, rows {}", again.not_sent(), rows.is_some()),
    );
    if let Some(rows) = rows {
        row_holder.append(&rows);
    }
    h.screenshot("not-sent").await;

    // An interruption stops and keeps (the session watch's lock).
    for entry in on_disk.of(CHAT) {
        on_disk.remove(&entry);
    }
    recording.start_voice(false);
    h.wait_until(WAIT, || recording.recording() == Recording::HandsFree)
        .await;
    h.sleep(Duration::from_millis(1300)).await;
    media::session_watch::interrupt(RecordingEnd::SessionLocked);
    let kept = h
        .wait_until(WAIT, || {
            recording.recording() == Recording::None && on_disk.of(CHAT).len() == 1
        })
        .await;
    h.check(
        "an_interruption_stops_and_keeps",
        kept && host.sent.borrow().len() == 1,
        format!(
            "{:?}, {} parked",
            recording.recording(),
            on_disk.of(CHAT).len()
        ),
    );
    for entry in on_disk.of(CHAT) {
        on_disk.remove(&entry);
    }
    while let Some(child) = row_holder.first_child() {
        row_holder.remove(&child);
    }

    // ---- a circle in place --------------------------------------------------------------------------------
    let round = AttachmentDto {
        id: CIRCLE_ID,
        kind: "video".to_string(),
        mime: Some("video/mp4".to_string()),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(3_000),
        has_preview: true,
        round: true,
        ..AttachmentDto::default()
    };
    let scroller = gtk::ScrolledWindow::builder()
        .height_request(300)
        .vexpand(true)
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (circle, circle_widget) = media::circle::Circle::new(&message, &round);
    list.append(&circle_widget);
    list.append(&gtk::Box::builder().height_request(1200).build());
    scroller.set_child(Some(&list));
    column.append(&scroller);
    h.sleep(Duration::from_millis(600)).await;
    circle.click();
    let playing = h
        .wait_until(WAIT, || circle.phase() == RoundPhase::Playing)
        .await;
    h.check(
        "circle_plays_in_place",
        playing,
        format!("{:?}", circle.phase()),
    );
    h.sleep(Duration::from_millis(900)).await;
    // Drawn once the frames are in the picture (a slow software renderer may need a moment).
    h.wait_until(Duration::from_secs(5), || {
        round_pixels(&circle_widget).is_ok_and(|(corner, centre)| corner == 0 && centre == 255)
    })
    .await;
    let clipped = round_pixels(&circle_widget);
    h.check(
        "circle_is_clipped_round",
        clipped
            .as_ref()
            .is_ok_and(|(corner, centre)| *corner == 0 && *centre == 255),
        format!("corner/centre alpha {clipped:?}"),
    );
    h.screenshot("circle-playing").await;
    let adjustment = scroller.vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    let stopped = h
        .wait_until(WAIT, || circle.phase() == RoundPhase::Rest)
        .await;
    h.check(
        "circle_stops_when_scrolled_away",
        stopped,
        format!("{:?}", circle.phase()),
    );
    column.remove(&scroller);
    drop(circle);
    window.set_visible(false);

    // ---- the video message recorder on the window's overlay ---------------------------------------------
    if let Some(main) = shell.main_window() {
        main.present();
        h.sleep(Duration::from_millis(400)).await;
        let video_host = Rc::new(TestHost {
            window: main.window().clone().upcast(),
            staged: RefCell::new(Vec::new()),
            sent: RefCell::new(Vec::new()),
            changes: Cell::new(0),
        });
        let video = shell.composer_slots.get().attach(video_host.clone());
        video.record_video();
        let layer = media::recorder_layer::layer();
        let preview = h
            .wait_until(WAIT, || {
                layer.as_ref().is_some_and(|layer| {
                    layer.stage() == RecorderStage::Preview && layer.has_picture()
                })
            })
            .await;
        h.check(
            "recorder_opens_on_the_overlay_with_the_test_camera",
            preview
                && main.recorder_open()
                && layer.as_ref().is_some_and(|layer| {
                    layer.widget().is_visible()
                        && layer.widget().parent().is_some_and(|parent| {
                            &parent == main.overlay().upcast_ref::<gtk::Widget>()
                        })
                }),
            format!(
                "{:?}",
                layer.as_ref().map(|layer| (layer.stage(), layer.trouble()))
            ),
        );
        h.sleep(Duration::from_millis(500)).await;
        h.screenshot("recorder-preview").await;
        if let Some(layer) = layer.as_ref() {
            layer.press_slot();
            h.wait_until(WAIT, || layer.stage() == RecorderStage::Recording)
                .await;
            h.sleep(Duration::from_millis(2600)).await;
            h.screenshot("recorder-recording").await;
            // Past the 600 ms guard: Stop.
            layer.press_slot();
            let review = h
                .wait_until(WAIT, || {
                    layer.stage() == RecorderStage::Review && layer.clip_ready()
                })
                .await;
            h.check(
                "recorder_reviews_the_take",
                review,
                format!("{:?}", layer.stage()),
            );
            h.screenshot("recorder-review").await;
            // §10.6 / §3.8: a quit over a take asks first — `Shell::close_guard` holds it and puts
            // "Delete video message?"; Keep stays in REVIEW and nothing quits.
            let held = !(shell.close_guard.get())();
            let asked = h
                .wait_until(WAIT, || media::recorder_layer::asking().is_some())
                .await;
            h.sleep(Duration::from_millis(500)).await;
            h.screenshot("recorder-quit-question").await;
            if let Some(question) = media::recorder_layer::asking() {
                question.close();
            }
            let kept = h
                .wait_until(WAIT, || {
                    media::recorder_layer::asking().is_none()
                        && layer.stage() == RecorderStage::Review
                })
                .await;
            h.check(
                "a_quit_over_a_take_asks_first",
                held && asked && kept && !shell.quitting(),
                format!(
                    "held {held}, asked {asked}, kept in review {kept}, quitting {}",
                    shell.quitting()
                ),
            );
            h.sleep(Duration::from_millis(700)).await;
            layer.press_slot();
            let sent = h
                .wait_until(WAIT, || !video_host.sent.borrow().is_empty())
                .await;
            let square_sent = video_host.sent.borrow().first().cloned();
            let reading = match &square_sent {
                Some(sent) => {
                    let file = folder.join("sent-square.mp4");
                    let _ = std::fs::write(&file, &sent.bytes);
                    media::video_recorder::read_take(&file).await
                }
                None => None,
            };
            h.check(
                "recorder_sends_a_480_square",
                sent && square_sent.as_ref().is_some_and(|sent| {
                    sent.kind == "video"
                        && sent.width == Some(480)
                        && sent.height == Some(480)
                        && sent
                            .preview
                            .as_ref()
                            .is_some_and(|p| p.starts_with(&[0xFF, 0xD8]))
                }) && reading.as_ref().is_some_and(|read| {
                    read.video_codec == "h264"
                        && read.width == 480
                        && read.height == 480
                        && read.audio_codec.as_deref() == Some("aac")
                        && read.moov_first
                }),
                format!("{reading:?}"),
            );
            let closed = h.wait_until(WAIT, || !main.recorder_open()).await;
            h.check(
                "recorder_closes_after_send",
                closed && !layer.widget().is_visible(),
                String::new(),
            );
        }
    }

    // ---- preparing a picked video: transcoded to the profile; a cancel stages nothing ----------------------
    let cancel = Cancel::new();
    let mut told = Vec::new();
    let picked = media::preparing::prepare_picked_told(
        &big,
        "video/mp4",
        Some("big.mp4"),
        &cancel,
        &mut |line| told.push(line.to_string()),
    )
    .await;
    let made = match &picked {
        Ok(Picked::Staged(media)) => {
            let file = folder.join("transcoded.mp4");
            let _ = std::fs::write(&file, &media.bytes);
            Some((file, media.width, media.height))
        }
        _ => None,
    };
    let reading = match &made {
        Some((file, _, _)) => media::preparing::read(file).await,
        None => None,
    };
    let cli = match &made {
        Some((file, _, _)) => {
            h.run(&["gst-discoverer-1.0", &file.to_string_lossy()])
                .await
                .1
        }
        None => String::new(),
    };
    let video = reading.as_ref().and_then(|reading| reading.video.clone());
    h.check(
        "picked_video_transcoded_to_the_profile",
        made.as_ref().is_some_and(|(file, width, height)| {
            *width == Some(1280)
                && *height == Some(720)
                && std::fs::read(file)
                    .ok()
                    .and_then(|bytes| faststart::is_moov_first(&bytes))
                    == Some(true)
        }) && video.as_ref().is_some_and(|video| {
            video.codec == "h264"
                && video.profile.as_deref() == Some("high")
                && video.width == 1280
                && video.height == 720
                && video.frame_rate_numerator <= 30 * video.frame_rate_denominator.max(1)
        }) && reading
            .as_ref()
            .is_some_and(|reading| reading.audio.first().is_some_and(|a| a.codec == "aac"))
            && cli.contains("H.264 (High Profile)"),
        format!(
            "{video:?}; discoverer: {}; told: {}",
            cli.lines()
                .filter(|l| l.contains("video") || l.contains("H.264"))
                .collect::<Vec<_>>()
                .join(" | "),
            told.join(" | ")
        ),
    );
    let cancel = Cancel::new();
    {
        let cancel = cancel.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || cancel.cancel());
    }
    let cancelled =
        media::preparing::prepare_picked(&long, "video/mp4", Some("long.mp4"), &cancel).await;
    let prepared_dir = shell.paths.cache_dir().join("prepared");
    let leftovers = std::fs::read_dir(&prepared_dir)
        .map(|dir| dir.count())
        .unwrap_or(0);
    h.check(
        "a_cancel_stages_nothing_and_leaves_nothing",
        cancelled.is_err() && leftovers == 0,
        format!(
            "{:?}, {leftovers} file(s) left",
            cancelled.as_ref().map(|_| "prepared")
        ),
    );

    // ---- an animation: the raw encode keeps its index last ---------------------------------------------------
    let gif = folder.join("anim.gif");
    let (drawn, _) = h
        .run(&[
            "convert",
            "-size",
            "64x48",
            "xc:red",
            "xc:blue",
            "xc:green",
            "-set",
            "delay",
            "10",
            "-loop",
            "0",
            &gif.to_string_lossy(),
        ])
        .await;
    let raw = if drawn {
        raw_animation(&gif, &shell.paths.tmp_dir()).await
    } else {
        None
    };
    let order = raw.as_deref().map(top_boxes).unwrap_or_default();
    let mdat = order.iter().position(|b| b == "mdat");
    let moov = order.iter().position(|b| b == "moov");
    h.check(
        "animation_raw_encode_has_no_faststart",
        order.first().map(String::as_str) == Some("ftyp")
            && order
                .iter()
                .skip(1)
                .find(|b| *b != "free")
                .map(String::as_str)
                == Some("mdat")
            && mdat.zip(moov).is_some_and(|(mdat, moov)| moov > mdat),
        format!("{order:?}"),
    );
    let staged = match std::fs::read(&gif) {
        Ok(bytes) => {
            let mut lines = Vec::new();
            let mut log = |line: &str| lines.push(line.to_string());
            media::animation::prepare(
                Rc::from(bytes.as_slice()),
                shell.clock.as_ref(),
                &mut log,
                &Cancel::new(),
            )
            .await
            .ok()
            .flatten()
        }
        Err(_) => None,
    };
    h.check(
        "animation_made_with_its_index_moved_in_front",
        staged.as_ref().is_some_and(|staged| {
            staged.animation
                && staged.kind == "video"
                && faststart::is_moov_first(&staged.bytes) == Some(true)
                && staged.preview.is_some()
        }),
        format!(
            "{:?}",
            staged
                .as_ref()
                .map(|s| (s.width, s.height, s.duration_ms, s.bytes.len()))
        ),
    );
    window.destroy();
    media::use_test_sources(false);
}

/// Runs each description with `gst-launch-1.0 -q -e`; true when all succeeded.
async fn make(h: &Harness, descriptions: &[&str]) -> bool {
    for description in descriptions {
        let mut argv = vec!["gst-launch-1.0", "-q", "-e"];
        argv.extend(description.split_whitespace());
        let (ok, output) = h.run(&argv).await;
        if !ok {
            eprintln!("selftest: gst-launch failed: {output}");
            return false;
        }
    }
    true
}

/// The alpha of the circle's corner and centre, rendered off the widget itself (§1.5's test).
fn round_pixels(widget: &gtk::Box) -> Result<(u8, u8), String> {
    // The clip is the circle's innermost `fc-round` overlay.
    let clip = find_round(widget.upcast_ref()).ok_or("no fc-round box")?;
    let (width, height) = (clip.width(), clip.height());
    if width <= 0 || height <= 0 {
        return Err(format!(
            "not allocated: {width}x{height}, mapped {}",
            clip.is_mapped()
        ));
    }
    let paintable = gtk::WidgetPaintable::new(Some(&clip));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node().ok_or("nothing drawn")?;
    let renderer = clip
        .native()
        .and_then(|native| native.renderer())
        .ok_or("no renderer")?;
    let texture = renderer.render_texture(
        &node,
        Some(&gtk::graphene::Rect::new(
            0.0,
            0.0,
            width as f32,
            height as f32,
        )),
    );
    let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
    downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let alpha = |x: usize, y: usize| bytes.get(y * stride + x * 4 + 3).copied();
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    Ok((
        alpha(1, 1).ok_or("no corner pixel")?,
        alpha(w / 2, h / 2).ok_or("no centre pixel")?,
    ))
}

fn find_round(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if widget.has_css_class("fc-round") {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_round(&current) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

/// The animation encoder's own output for a GIF (before fc-logic's timing and faststart).
async fn raw_animation(gif: &Path, tmp: &Path) -> Option<Vec<u8>> {
    let bytes: Rc<[u8]> = Rc::from(std::fs::read(gif).ok()?.as_slice());
    let layout = animation_prep::layout(&bytes)?;
    let plan = animation_timing::plan(
        i64::from(layout.width),
        i64::from(layout.height),
        &layout.stated_ms(),
    )?;
    let source = animation_preparing::frame_source(
        Rc::clone(&bytes),
        &layout,
        Rc::new(media::animation::GdkStillDecoder),
    )?;
    let cancel = Cancel::new();
    let mut feed = FrameFeed::new(&source, plan.clone(), cancel.clone())?;
    let encoder = media::animation::GstAnimationEncoder::new(PathBuf::from(tmp));
    match encoder
        .encode_raw(&plan, H264Profile::High, &mut feed, cancel)
        .await
    {
        Ok(Attempted::Taken(bytes)) => Some(bytes),
        _ => None,
    }
}

/// An MP4's top-level box types, in order.
fn top_boxes(file: &[u8]) -> Vec<String> {
    let mut at = 0usize;
    let mut out = Vec::new();
    while at + 8 <= file.len() {
        let size =
            u32::from_be_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]) as usize;
        out.push(String::from_utf8_lossy(&file[at + 4..at + 8]).into_owned());
        let size = if size == 1 && at + 16 <= file.len() {
            u64::from_be_bytes(file[at + 8..at + 16].try_into().unwrap_or([0; 8])) as usize
        } else if size == 0 {
            file.len() - at
        } else {
            size
        };
        if size < 8 {
            break;
        }
        at += size;
    }
    out
}
