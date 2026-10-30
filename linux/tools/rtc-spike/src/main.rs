//! Phase-0 spike for issue #112: a GStreamer webrtcbin peer that sends Opus + VP8 and counts
//! what it decodes from the other side. Signalling is JSON over one WebSocket it serves.
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use gst::prelude::*;
use serde_json::{json, Value};

const PIPELINE: &str = "webrtcbin name=w bundle-policy=max-bundle \
  audiotestsrc is-live=true wave=sine ! audioconvert ! audioresample ! opusenc ! rtpopuspay pt=111 ! \
    application/x-rtp,media=audio,encoding-name=OPUS,payload=111 ! w. \
  videotestsrc is-live=true pattern=ball ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert ! \
    vp8enc deadline=1 keyframe-max-dist=30 ! rtpvp8pay pt=96 ! \
    application/x-rtp,media=video,encoding-name=VP8,payload=96 ! w.";

fn description(kind: gst_webrtc::WebRTCSDPType, sdp: &str) -> anyhow::Result<gst_webrtc::WebRTCSessionDescription> {
    let message = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())?;
    Ok(gst_webrtc::WebRTCSessionDescription::new(kind, message))
}

fn main() -> anyhow::Result<()> {
    let offers = std::env::args().nth(1).as_deref() == Some("offer");
    gst::init()?;
    println!("gstreamer {}", gst::version_string());

    let listener = TcpListener::bind("0.0.0.0:9000")?;
    println!("listening; this side {}", if offers { "OFFERS" } else { "ANSWERS" });
    let (stream, peer) = listener.accept()?;
    println!("browser connected from {peer}");
    let mut ws = tungstenite::accept(stream)?;
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(20)))?;

    let pipeline = gst::parse::launch(PIPELINE)?.downcast::<gst::Pipeline>().unwrap();
    let w = pipeline.by_name("w").unwrap();
    let (out_tx, out_rx) = mpsc::channel::<String>();
    let audio = Arc::new(AtomicU64::new(0));
    let video = Arc::new(AtomicU64::new(0));

    {
        let tx = out_tx.clone();
        w.connect("on-ice-candidate", false, move |values| {
            let mline = values[1].get::<u32>().unwrap();
            let candidate = values[2].get::<String>().unwrap();
            let _ = tx.send(json!({"type": "ice", "sdpMLineIndex": mline, "candidate": candidate}).to_string());
            None
        });
    }
    if offers {
        let tx = out_tx.clone();
        w.connect("on-negotiation-needed", false, move |values| {
            let w = values[0].get::<gst::Element>().unwrap();
            let (w2, tx) = (w.clone(), tx.clone());
            let promise = gst::Promise::with_change_func(move |reply| {
                let reply = reply.unwrap().unwrap();
                let offer = reply.value("offer").unwrap().get::<gst_webrtc::WebRTCSessionDescription>().unwrap();
                w2.emit_by_name::<()>("set-local-description", &[&offer, &None::<gst::Promise>]);
                let _ = tx.send(json!({"type": "offer", "sdp": offer.sdp().as_text().unwrap()}).to_string());
            });
            w.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
            None
        });
    }
    {
        let (pipe, audio, video) = (pipeline.clone(), audio.clone(), video.clone());
        w.connect_pad_added(move |_, pad| {
            if pad.direction() != gst::PadDirection::Src {
                return;
            }
            let decodebin = gst::ElementFactory::make("decodebin").build().unwrap();
            let (pipe2, audio, video) = (pipe.clone(), audio.clone(), video.clone());
            decodebin.connect_pad_added(move |_, dpad| {
                let caps = dpad.current_caps().unwrap();
                let name = caps.structure(0).unwrap().name().to_string();
                println!("decoding {name}");
                let counter = if name.starts_with("audio") { audio.clone() } else { video.clone() };
                let sink = gst::ElementFactory::make("fakesink").property("sync", false).build().unwrap();
                pipe2.add(&sink).unwrap();
                sink.sync_state_with_parent().unwrap();
                let sink_pad = sink.static_pad("sink").unwrap();
                dpad.link(&sink_pad).unwrap();
                sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    gst::PadProbeReturn::Ok
                });
            });
            pipe.add(&decodebin).unwrap();
            decodebin.sync_state_with_parent().unwrap();
            pad.link(&decodebin.static_pad("sink").unwrap()).unwrap();
        });
    }
    {
        let tx = out_tx.clone();
        w.connect_notify(Some("ice-connection-state"), move |w, _| {
            let state = w.property::<gst_webrtc::WebRTCICEConnectionState>("ice-connection-state");
            let _ = tx.send(json!({"type": "log", "text": format!("gst ice {state:?}")}).to_string());
        });
    }
    pipeline.set_state(gst::State::Playing)?;

    let bus = pipeline.bus().unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        while let Ok(text) = out_rx.try_recv() {
            if text.contains("\"log\"") {
                println!("{text}");
                continue;
            }
            ws.send(tungstenite::Message::text(text))?;
        }
        while let Some(message) = bus.pop() {
            if let gst::MessageView::Error(err) = message.view() {
                anyhow::bail!("pipeline error from {:?}: {} ({:?})", err.src().map(|s| s.path_string()), err.error(), err.debug());
            }
        }
        match ws.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let value: Value = serde_json::from_str(&text)?;
                match value["type"].as_str() {
                    Some("offer") => {
                        let offer = description(gst_webrtc::WebRTCSDPType::Offer, value["sdp"].as_str().unwrap())?;
                        let (w2, tx) = (w.clone(), out_tx.clone());
                        let promise = gst::Promise::with_change_func(move |_| {
                            let w3 = w2.clone();
                            let answer_promise = gst::Promise::with_change_func(move |reply| {
                                let reply = reply.unwrap().unwrap();
                                let answer = reply.value("answer").unwrap().get::<gst_webrtc::WebRTCSessionDescription>().unwrap();
                                w3.emit_by_name::<()>("set-local-description", &[&answer, &None::<gst::Promise>]);
                                let _ = tx.send(json!({"type": "answer", "sdp": answer.sdp().as_text().unwrap()}).to_string());
                            });
                            w2.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &answer_promise]);
                        });
                        w.emit_by_name::<()>("set-remote-description", &[&offer, &promise]);
                    }
                    Some("answer") => {
                        let answer = description(gst_webrtc::WebRTCSDPType::Answer, value["sdp"].as_str().unwrap())?;
                        w.emit_by_name::<()>("set-remote-description", &[&answer, &None::<gst::Promise>]);
                    }
                    Some("ice") => {
                        let mline = value["sdpMLineIndex"].as_u64().unwrap_or(0) as u32;
                        let candidate = value["candidate"].as_str().unwrap_or_default().to_string();
                        if !candidate.is_empty() {
                            w.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
                        }
                    }
                    Some("log") => println!("browser: {}", value["text"]),
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let (a, v) = (audio.load(Ordering::Relaxed), video.load(Ordering::Relaxed));
    println!("GST RECEIVED: audio buffers {a}, video frames {v}");
    pipeline.set_state(gst::State::Null)?;
    if a > 50 && v > 50 { println!("GST SIDE OK"); Ok(()) } else { anyhow::bail!("too little media decoded") }
}
