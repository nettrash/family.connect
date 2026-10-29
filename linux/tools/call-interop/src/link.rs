//! The peer's signalling socket: a WebSocket to the relay on a thread of its own (a tool may;
//! the app's socket is libsoup's on the main context), frames in through a `futures_channel`
//! the main context drains, frames out through a channel the thread writes from.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use futures_channel::mpsc::{UnboundedReceiver, unbounded};

/// `host:port` of a `ws://host:port/…` URL.
pub fn address_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("ws://")?;
    let address = rest.split(['/', '?']).next()?;
    if address.is_empty() {
        return None;
    }
    Some(if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:80")
    })
}

/// Connect (retrying for up to ten seconds — the relay may still be starting) and keep the socket
/// on a thread: what goes into the returned sender is written, what arrives comes out of the
/// receiver. The receiver ends when the socket does.
pub fn open(url: &str) -> (mpsc::Sender<String>, UnboundedReceiver<String>) {
    let (out, outgoing) = mpsc::channel::<String>();
    let (incoming, frames) = unbounded::<String>();
    let url = url.to_string();
    std::thread::spawn(move || {
        let Some(address) = address_of(&url) else {
            eprintln!("link: not a ws:// URL");
            return;
        };
        let started = Instant::now();
        let mut socket = loop {
            let connected = TcpStream::connect(&address)
                .ok()
                .and_then(|stream| tungstenite::client(url.as_str(), stream).ok());
            match connected {
                Some((socket, _)) => break socket,
                None if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                None => {
                    eprintln!("link: the relay did not answer");
                    return;
                }
            }
        };
        let _ = socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(10)));
        loop {
            while let Ok(text) = outgoing.try_recv() {
                if socket.send(tungstenite::Message::text(text)).is_err() {
                    return;
                }
            }
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    if incoming.unbounded_send(text.as_str().to_string()).is_err() {
                        return;
                    }
                }
                Ok(tungstenite::Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => return,
            }
        }
    });
    (out, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_the_host_and_port() {
        assert_eq!(
            address_of("ws://relay:9000/?user=1").as_deref(),
            Some("relay:9000")
        );
        assert_eq!(address_of("ws://relay?user=1").as_deref(), Some("relay:80"));
        assert_eq!(address_of("http://relay:9000"), None);
        assert_eq!(address_of("ws:///x"), None);
    }
}
