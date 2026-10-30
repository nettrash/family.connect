//! An SDP in one line — each m-section's kind, direction and codecs — for the harness's report.
//! Never its addresses, fingerprints or credentials.

#[derive(Default)]
struct Section {
    kind: String,
    port_zero: bool,
    bundle_only: bool,
    payloads: Vec<String>,
    names: Vec<(String, String)>,
    direction: Option<String>,
}

impl Section {
    fn line(&self) -> String {
        let codecs: Vec<String> = self
            .payloads
            .iter()
            .filter_map(|payload| {
                self.names
                    .iter()
                    .find(|(pt, _)| pt == payload)
                    .map(|(pt, name)| format!("{name}/{pt}"))
            })
            .filter(|codec| {
                let upper = codec.to_ascii_uppercase();
                ["OPUS", "VP8", "VP9", "H264", "AV1"]
                    .iter()
                    .any(|known| upper.starts_with(known))
            })
            .collect();
        let direction = self.direction.as_deref().unwrap_or("sendrecv");
        // A bundled m-line after the first is port 0 with `a=bundle-only` (JSEP max-bundle) —
        // not a rejection.
        let state = match (self.port_zero, self.bundle_only) {
            (true, true) => format!("{direction}(bundle-only)"),
            (true, false) => "rejected".to_string(),
            _ => direction.to_string(),
        };
        format!("{} {state} {}", self.kind, codecs.join(","))
    }
}

/// `audio sendrecv OPUS/111 | video recvonly VP8/96`.
pub fn summary(sdp: &str) -> String {
    let mut sections: Vec<Section> = Vec::new();
    for line in sdp.lines().map(|line| line.trim_end_matches('\r')) {
        if let Some(media) = line.strip_prefix("m=") {
            let mut words = media.split(' ');
            let kind = words.next().unwrap_or_default().to_string();
            let port_zero = words.next() == Some("0");
            let payloads = words.skip(1).map(str::to_string).collect();
            sections.push(Section {
                kind,
                port_zero,
                payloads,
                ..Section::default()
            });
        } else if let Some(section) = sections.last_mut() {
            if let Some(map) = line.strip_prefix("a=rtpmap:") {
                if let Some((pt, format)) = map.split_once(' ') {
                    let name = format.split('/').next().unwrap_or_default();
                    section.names.push((pt.to_string(), name.to_string()));
                }
            } else if line == "a=bundle-only" {
                section.bundle_only = true;
            } else if let Some(state) = line
                .strip_prefix("a=")
                .filter(|state| matches!(*state, "sendrecv" | "sendonly" | "recvonly" | "inactive"))
            {
                section.direction = Some(state.to_string());
            }
        }
    }
    sections
        .iter()
        .map(Section::line)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The video codec of a summary (`VP8`, `H264`), when its video m-line carries one.
pub fn video_codec(summary: &str) -> Option<String> {
    let video = summary
        .split(" | ")
        .find(|section| section.starts_with("video "))?;
    let codecs = video.split(' ').nth(2)?;
    let first = codecs.split(',').next()?;
    let name = first.split('/').next()?;
    (!name.is_empty()).then(|| name.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_names_each_section_its_direction_and_codecs() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 109 101\r\na=sendrecv\r\n\
                   a=rtpmap:109 opus/48000/2\r\na=rtpmap:101 telephone-event/8000\r\n\
                   a=candidate:1 1 udp 1 10.0.0.1 9 typ host\r\n\
                   m=video 9 UDP/TLS/RTP/SAVPF 120 121\r\na=recvonly\r\n\
                   a=rtpmap:120 VP8/90000\r\na=rtpmap:121 VP9/90000\r\n\
                   m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n\
                   m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=bundle-only\r\na=rtpmap:96 VP8/90000\r\n";
        assert_eq!(
            summary(sdp),
            "audio sendrecv opus/109 | video recvonly VP8/120,VP9/121 | video rejected VP8/96 \
             | video sendrecv(bundle-only) VP8/96"
        );
        assert!(!summary(sdp).contains("10.0.0.1"));
    }

    #[test]
    fn the_video_codec_is_the_first_of_the_video_section() {
        assert_eq!(
            video_codec("audio sendrecv OPUS/111 | video sendrecv H264/106").as_deref(),
            Some("H264")
        );
        assert_eq!(video_codec("audio sendrecv OPUS/111"), None);
    }
}
