//! The call interop harness (issue #112, package P6) — see `run.sh` for the matrix it runs.
//!
//! ```text
//! call-interop relay [PORT]          the server's part of a call (default port 9000)
//! call-interop peer [FLAGS]          a Linux peer: CallEngine + GstCallMedia (peer.rs's flags)
//! call-interop answer-fixture FILE …  an offer from a file answered by GstCallMedia (fixture.rs)
//! ```
//!
//! The media is the APP's: the four files of `app/src/calls/` that make it (`ice`, `media`,
//! `pipeline`, `tones`) compiled into this binary as they are, so what is proven here is the code
//! the app ships. Not the folder's `mod.rs`: its other half is the app's shell (the Call window,
//! the routes), which needs the app around it.

#[path = "../../../app/src/calls"]
#[allow(dead_code, unused_imports)]
mod calls {
    pub mod ice;
    mod media;
    pub mod pipeline;
    pub mod tones;

    pub use ice::{AgentFacts, SilenceRule};
    pub use media::{CallCamera, CallDevices, CallStats, CameraFeed, GstCallMedia, NoCamera};
    pub use pipeline::VideoCodec;
}

mod fixture;
mod link;
mod peer;
mod recording;
mod relay;
mod sdp_summary;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("relay") => {
            let port = args
                .get(1)
                .and_then(|port| port.parse().ok())
                .unwrap_or(9000);
            match relay::serve(port) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("relay: {error}");
                    2
                }
            }
        }
        Some("answer-fixture") => fixture::run(&args[1..]),
        Some("offer") => fixture::offer(&args[1..]),
        Some("peer") => match peer::Options::parse(&args[1..]) {
            Ok(options) => peer::run(options),
            Err(error) => {
                eprintln!("peer: {error}");
                2
            }
        },
        _ => {
            eprintln!(
                "usage: call-interop relay [PORT] | peer [FLAGS] | answer-fixture FILE [FLAGS]"
            );
            2
        }
    };
    std::process::exit(code);
}
