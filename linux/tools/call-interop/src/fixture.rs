//! An offer from a file, answered by `GstCallMedia` with no far side at all: what the media
//! makes of a peer's SDP that no browser here can send (an H.264-only offer — Playwright's
//! Chromium and Firefox carry no H.264) or that should be pinned (Firefox's payload numbers, the
//! web client's old voice call with a `recvonly` video m-line).
//!
//! ```text
//! call-interop answer-fixture FILE [--video] [--no-camera] [--expect-codec C]
//!     [--expect-audio-pt N] [--expect-video-pt N] [--expect-video DIRECTION]
//! ```

use std::cell::Cell;
use std::rc::Rc;

use fc_logic::calls::CallMedia;

use crate::calls::{CallDevices, GstCallMedia, NoCamera};
use crate::sdp_summary::{summary, video_codec};

pub fn run(args: &[String]) -> i32 {
    let Some(file) = args.first() else {
        eprintln!("answer-fixture FILE …");
        return 2;
    };
    let Ok(offer) = std::fs::read_to_string(file) else {
        eprintln!("answer-fixture: cannot read {file}");
        return 2;
    };
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    if gst::init().is_err() {
        return 2;
    }
    let video = flag("--video");
    let mut devices = CallDevices::from_env();
    if flag("--no-camera") {
        devices.camera = None;
    }
    let media = Rc::new(GstCallMedia::new(devices, Rc::new(NoCamera)));
    let main = glib::MainLoop::new(None, false);
    let answer: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    let refused = Rc::new(Cell::new(None::<&'static str>));
    {
        let (media, main, answer, refused, offer) = (
            media.clone(),
            main.clone(),
            answer.clone(),
            refused.clone(),
            offer.clone(),
        );
        glib::MainContext::default().spawn_local(async move {
            let grant = media.open(video).await;
            println!("fixture: open → {grant:?}");
            if !media.connect(Vec::new(), false).await {
                refused.set(Some("connect"));
            } else if !media.set_remote(true, offer).await {
                refused.set(Some("set_remote"));
            } else {
                match media.create_answer().await {
                    Some(sdp) => *answer.borrow_mut() = Some(sdp),
                    None => refused.set(Some("create_answer")),
                }
            }
            media.close();
            main.quit();
        });
    }
    main.run();
    println!("fixture: offer  {}", summary(&offer));
    let Some(answer) = answer.borrow().clone() else {
        println!(
            "fixture: CHECK FAIL: no answer ({})",
            refused.get().unwrap_or("?")
        );
        println!("fixture: RESULT FAIL");
        return 1;
    };
    let line = summary(&answer);
    println!("fixture: answer {line}");
    let mut failed = false;
    let mut check = |name: &str, passed: bool, detail: String| {
        failed |= !passed;
        println!(
            "fixture: CHECK {}: {name} — {detail}",
            if passed { "PASS" } else { "FAIL" }
        );
    };
    let section = |kind: &str| {
        line.split(" | ")
            .find(|section| section.starts_with(&format!("{kind} ")))
            .unwrap_or_default()
            .to_string()
    };
    if let Some(codec) = value("--expect-codec") {
        let got = video_codec(&line);
        check(
            "answered video codec",
            got.as_deref() == Some(codec.to_ascii_uppercase().as_str()),
            format!("{got:?}"),
        );
    }
    if let Some(pt) = value("--expect-audio-pt") {
        let audio = section("audio");
        check(
            "answered audio payload",
            audio.ends_with(&format!("/{pt}")) || audio.contains(&format!("/{pt},")),
            audio,
        );
    }
    if let Some(pt) = value("--expect-video-pt") {
        let video = section("video");
        check(
            "answered video payload",
            video.contains(&format!("/{pt}")),
            video,
        );
    }
    if let Some(direction) = value("--expect-video") {
        let video = section("video");
        check(
            "answered video direction",
            video
                .split(' ')
                .nth(1)
                .is_some_and(|state| state.starts_with(&direction)),
            video,
        );
    }
    println!("fixture: RESULT {}", if failed { "FAIL" } else { "PASS" });
    i32::from(failed)
}

/// `call-interop offer [--video]`: the SDP `GstCallMedia` offers, printed whole — to read what a
/// far side is given.
pub fn offer(args: &[String]) -> i32 {
    if gst::init().is_err() {
        return 2;
    }
    let video = args.iter().any(|arg| arg == "--video");
    let media = Rc::new(GstCallMedia::new(
        CallDevices::from_env(),
        Rc::new(NoCamera),
    ));
    let main = glib::MainLoop::new(None, false);
    let offer: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    {
        let (media, main, offer) = (media.clone(), main.clone(), offer.clone());
        glib::MainContext::default().spawn_local(async move {
            let _ = media.open(video).await;
            if media.connect(Vec::new(), false).await {
                *offer.borrow_mut() = media.create_offer().await;
            }
            media.close();
            main.quit();
        });
    }
    main.run();
    match offer.borrow().as_deref() {
        Some(sdp) => {
            print!("{sdp}");
            0
        }
        None => 1,
    }
}
