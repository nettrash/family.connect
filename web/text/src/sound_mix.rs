//! A video's sound tracks MIXED into one (docs/protocol.md, "Preparing media
//! before upload", Rule E, issue #98).
//!
//! The profile allows one AAC-LC track, and a source can carry more that
//! play at once: a screen recording's app sound and its microphone, a camera
//! with two microphones. A transcode that wrote the first alone would drop a
//! voice, so every client mixes them: a plain sum of the samples, clamped to
//! full scale — what Apple's `AVAssetReaderAudioMixOutput` does — stereo when
//! any of them is, at the highest rate any of them has, each at its own place
//! in time.
//!
//! The browser decodes each track with its own `AudioDecoder` and hands the
//! decoded sound here as it comes, track by track and in whatever pieces the
//! decoders put out; [`Mixer`] says how much of the mix can be made from what
//! it has so far, makes it, and lets go of what no later frame needs. That
//! keeps a long clip's memory to what the decoders are ahead of one another
//! by — not the whole of every track decoded at once. It is plain arithmetic,
//! so it is here, tested natively, and comes out the same in every browser:
//! the same pieces in any order of arrival make the same mix, to the bit.
//!
//! A track whose rate is not the mix's is resampled by linear interpolation
//! — exact for a rate that divides the mix's, and for one that is the mix's
//! own (the usual case: both tracks of a screen recording at 48 kHz) no
//! interpolation at all, every sample copied as it was.

use crate::mp4_read::{Movie, Track};

/// Where one track's decoded sound sits in the mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    /// The rate its decoder hands out — which for HE-AAC is twice the rate
    /// its index states.
    pub rate: u32,
    /// 1 or 2.
    pub channels: u32,
    /// Frames of the MIX before it starts: its lead, the empty edit in front
    /// of its media.
    pub lead: u64,
    /// Its own frames before what it presents: its encoder's priming, which
    /// its edit list starts past.
    pub skip: u64,
    /// Its own frames it presents from `skip`; None: all of them.
    pub length: Option<u64>,
}

/// The rate and channels of the mix of voices decoded at these
/// `(rate, channels)`: the highest rate, and stereo when any of them is.
/// None for no voices, a rate of 0, or a voice of more than two channels —
/// which a transcode does not down-mix (see
/// [`crate::transcode::audio_route`]) and so does not mix either.
pub fn mix_format(voices: &[(u32, u32)]) -> Option<(u32, u32)> {
    if voices.is_empty()
        || voices
            .iter()
            .any(|&(rate, channels)| rate == 0 || !(1..=2).contains(&channels))
    {
        return None;
    }
    let rate = voices.iter().map(|&(rate, _)| rate).max()?;
    let channels = voices.iter().map(|&(_, channels)| channels).max()?;
    Some((rate, channels))
}

/// Where `track` of `movie` sits in a mix at `mix_rate`, decoded at `rate`
/// with `channels`: its lead (from the movie's timescale), and the priming
/// and presented length its edit list states (from its own) — the same
/// reading of the edit the single-track path makes, so that a voice mixed
/// lands where it would have alone. None where the edit cannot be followed
/// (see [`Track::edit_supported`]) or a timescale is 0 that has to divide.
pub fn voice(
    movie: &Movie,
    track: &Track,
    rate: u32,
    channels: u32,
    mix_rate: u32,
) -> Option<Voice> {
    if !track.edit_supported() || track.timescale == 0 || rate == 0 {
        return None;
    }
    let timescale = u128::from(track.timescale);
    let edit = track.edit.unwrap_or_default();
    let lead = if edit.lead == 0 {
        0
    } else {
        let movie_scale = u128::from(movie.timescale);
        if movie_scale == 0 {
            return None;
        }
        // To the nearest frame.
        (u128::from(edit.lead) * u128::from(mix_rate) + movie_scale / 2) / movie_scale
    };
    let start = u128::try_from(edit.media_start.max(0)).ok()?;
    let skip = start * u128::from(rate) / timescale;
    // The presented length is stated in the MOVIE's timescale.
    let length = match track.edit.and_then(|edit| edit.length) {
        Some(length) if movie.timescale > 0 => {
            let ticks = u128::from(length) * timescale / u128::from(movie.timescale);
            Some(u64::try_from(ticks * u128::from(rate) / timescale).ok()?)
        }
        _ => None,
    };
    Some(Voice {
        rate,
        channels,
        lead: u64::try_from(lead).ok()?,
        skip: u64::try_from(skip).ok()?,
        length,
    })
}

/// One voice's decoded sound not yet mixed, and where it stands.
struct Lane {
    voice: Voice,
    /// A plane per channel; `planes[c][0]` is the voice's own frame `base`.
    planes: Vec<Vec<f32>>,
    base: u64,
    /// Frames handed in, in all.
    total: u64,
    finished: bool,
}

impl Lane {
    /// The frame of this voice under mix frame `at`, and how far towards
    /// the next one, in `1 / mix_rate`s — or None where the voice is silent:
    /// before its lead, or past what it presents.
    fn under(&self, at: u64, mix_rate: u32) -> Option<(u64, u64)> {
        let voice = &self.voice;
        let into = at.checked_sub(voice.lead)?;
        let scaled = u128::from(into) * u128::from(voice.rate);
        let mix = u128::from(mix_rate);
        if let Some(length) = voice.length {
            if scaled >= u128::from(length) * mix {
                return None;
            }
        }
        let frame = u64::try_from(scaled / mix).ok()?.checked_add(voice.skip)?;
        Some((frame, u64::try_from(scaled % mix).ok()?))
    }

    /// The first mix frame this voice cannot make yet — every one before it
    /// it can — or None when it can make them all: it has finished, or
    /// everything it still has to give is past what it presents.
    fn frontier(&self, mix_rate: u32) -> Option<u64> {
        if self.finished {
            return None;
        }
        let voice = &self.voice;
        let held = self.base + self.planes[0].len() as u64;
        // Mix frame j needs own frames i and i + 1 (i = skip + into·rate/mix):
        // it can be made when i + 1 < held, that is when
        // into·rate < (held − skip − 1)·mix.
        let room = held.saturating_sub(voice.skip + 1);
        let into = (u128::from(room) * u128::from(mix_rate)).div_ceil(u128::from(voice.rate));
        let into = u64::try_from(into).ok()?;
        if let Some(length) = voice.length {
            let presented =
                (u128::from(length) * u128::from(mix_rate)).div_ceil(u128::from(voice.rate));
            if u128::from(into) >= presented {
                return None;
            }
        }
        voice.lead.checked_add(into)
    }

    /// Own frame `index`, channel `channel`: what was handed in, and 0 past
    /// its end.
    fn sample(&self, channel: usize, index: u64) -> f32 {
        index
            .checked_sub(self.base)
            .and_then(|at| self.planes[channel].get(usize::try_from(at).ok()?))
            .copied()
            .unwrap_or(0.0)
    }

    /// The mix frame after the last this voice sounds in — known only once
    /// it has finished.
    fn end(&self, mix_rate: u32) -> u64 {
        let voice = &self.voice;
        let mut presented = self.total.saturating_sub(voice.skip);
        if let Some(length) = voice.length {
            presented = presented.min(length);
        }
        let frames =
            (u128::from(presented) * u128::from(mix_rate)).div_ceil(u128::from(voice.rate));
        voice
            .lead
            .saturating_add(u64::try_from(frames).unwrap_or(u64::MAX))
    }
}

/// Several voices being mixed into one, as their sound arrives.
pub struct Mixer {
    rate: u32,
    channels: u32,
    lanes: Vec<Lane>,
    /// The next mix frame to make.
    at: u64,
}

impl Mixer {
    /// A mix of `voices` at `rate` and `channels` (see [`mix_format`]).
    /// None for no voices, a rate of 0, or a voice the mix cannot hold.
    pub fn new(rate: u32, channels: u32, voices: &[Voice]) -> Option<Self> {
        if voices.is_empty()
            || rate == 0
            || !(1..=2).contains(&channels)
            || voices
                .iter()
                .any(|voice| voice.rate == 0 || !(1..=channels).contains(&voice.channels))
        {
            return None;
        }
        Some(Mixer {
            rate,
            channels,
            lanes: voices
                .iter()
                .map(|&voice| Lane {
                    voice,
                    planes: vec![Vec::new(); voice.channels as usize],
                    base: 0,
                    total: 0,
                    finished: false,
                })
                .collect(),
            at: 0,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// The next decoded sound of voice `voice`: a plane per channel, all of
    /// one length. False — and nothing taken — when it is not that: the
    /// wrong number of planes, planes of different lengths, or a voice that
    /// has finished or does not exist.
    pub fn push(&mut self, voice: usize, planes: &[&[f32]]) -> bool {
        let Some(lane) = self.lanes.get_mut(voice) else {
            return false;
        };
        let frames = planes.first().map_or(0, |plane| plane.len());
        if lane.finished
            || planes.len() != lane.planes.len()
            || planes.iter().any(|plane| plane.len() != frames)
        {
            return false;
        }
        for (held, plane) in lane.planes.iter_mut().zip(planes) {
            held.extend_from_slice(plane);
        }
        lane.total += frames as u64;
        // What comes before where it is needed goes at once: a voice whose
        // edit starts a minute into its media would otherwise count that
        // minute as decoded ahead, be given no more, and hold the mix back
        // for ever.
        self.let_go();
        true
    }

    /// Voice `voice` has handed in all it has.
    pub fn finish(&mut self, voice: usize) {
        if let Some(lane) = self.lanes.get_mut(voice) {
            lane.finished = true;
        }
    }

    /// Frames of voice `voice` handed in and not yet let go of — what its
    /// decoder is ahead of the mix by.
    pub fn buffered(&self, voice: usize) -> u64 {
        self.lanes
            .get(voice)
            .map_or(0, |lane| lane.planes[0].len() as u64)
    }

    /// Whether every voice has finished.
    pub fn finished(&self) -> bool {
        self.lanes.iter().all(|lane| lane.finished)
    }

    /// The mix's length, in frames, once every voice has finished: to the
    /// end of the one that sounds last.
    pub fn length(&self) -> Option<u64> {
        self.finished().then(|| {
            self.lanes
                .iter()
                .map(|lane| lane.end(self.rate))
                .max()
                .unwrap_or(0)
        })
    }

    /// How many frames can be made now, from what has been handed in.
    pub fn ready(&self) -> u64 {
        let until = match self.length() {
            Some(length) => length,
            None => self
                .lanes
                .iter()
                .filter_map(|lane| lane.frontier(self.rate))
                .min()
                .unwrap_or(u64::MAX),
        };
        until.saturating_sub(self.at)
    }

    /// Whether the whole mix has been made.
    pub fn done(&self) -> bool {
        self.length().is_some_and(|length| self.at >= length)
    }

    /// Up to `frames` more of the mix — no more than [`Mixer::ready`] — as a
    /// plane per channel: every voice summed, a mono one into both channels
    /// of a stereo mix, and clamped to full scale.
    pub fn take(&mut self, frames: u64) -> Vec<Vec<f32>> {
        let count = frames.min(self.ready());
        let length = usize::try_from(count).unwrap_or(0);
        let mut out = vec![vec![0.0f32; length]; self.channels as usize];
        for lane in &self.lanes {
            for (offset, at) in (self.at..self.at + count).enumerate() {
                let Some((frame, towards)) = lane.under(at, self.rate) else {
                    continue;
                };
                for (channel, plane) in out.iter_mut().enumerate() {
                    // A mono voice sounds in every channel of the mix.
                    let from = channel.min(lane.planes.len() - 1);
                    let here = lane.sample(from, frame);
                    let value = if towards == 0 {
                        here
                    } else {
                        let next = lane.sample(from, frame + 1);
                        here + (next - here) * (towards as f32 / self.rate as f32)
                    };
                    plane[offset] += value;
                }
            }
        }
        for plane in &mut out {
            for sample in plane.iter_mut() {
                *sample = sample.clamp(-1.0, 1.0);
            }
        }
        self.at += count;
        self.let_go();
        out
    }

    /// Let go of every voice's frames that no frame from here on needs.
    fn let_go(&mut self) {
        let (at, rate) = (self.at, self.rate);
        for lane in &mut self.lanes {
            let needed = match lane.under(at, rate) {
                Some((frame, _)) => frame,
                // Before its lead: from where it starts. Past what it
                // presents: nothing more.
                None if at < lane.voice.lead => lane.voice.skip,
                None => u64::MAX,
            };
            let held = lane.base + lane.planes[0].len() as u64;
            let until = needed.min(held);
            if until > lane.base {
                let drop = (until - lane.base) as usize;
                for plane in &mut lane.planes {
                    plane.drain(..drop);
                }
                lane.base = until;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4_read::Edit;

    fn plain(rate: u32, channels: u32) -> Voice {
        Voice {
            rate,
            channels,
            lead: 0,
            skip: 0,
            length: None,
        }
    }

    /// Everything a mixer will make, `pieces` frames at a time.
    fn drain(mixer: &mut Mixer, pieces: u64) -> Vec<Vec<f32>> {
        let mut out = vec![Vec::new(); mixer.channels() as usize];
        while !mixer.done() {
            let block = mixer.take(pieces);
            assert!(!block[0].is_empty(), "a finished mix that cannot go on");
            for (all, part) in out.iter_mut().zip(block) {
                all.extend(part);
            }
        }
        out
    }

    /// Each voice's planes handed in whole, then finished.
    fn mixed(rate: u32, channels: u32, voices: &[(Voice, Vec<Vec<f32>>)]) -> Vec<Vec<f32>> {
        let list: Vec<Voice> = voices.iter().map(|(voice, _)| *voice).collect();
        let mut mixer = Mixer::new(rate, channels, &list).unwrap();
        for (index, (_, planes)) in voices.iter().enumerate() {
            let planes: Vec<&[f32]> = planes.iter().map(Vec::as_slice).collect();
            assert!(mixer.push(index, &planes));
            mixer.finish(index);
        }
        drain(&mut mixer, 1000)
    }

    #[test]
    fn the_format_is_the_highest_rate_and_stereo_when_any_voice_is() {
        assert_eq!(mix_format(&[(48_000, 1), (48_000, 1)]), Some((48_000, 1)));
        assert_eq!(mix_format(&[(44_100, 1), (48_000, 2)]), Some((48_000, 2)));
        assert_eq!(mix_format(&[(96_000, 2), (16_000, 1)]), Some((96_000, 2)));
        assert_eq!(mix_format(&[]), None);
        assert_eq!(mix_format(&[(48_000, 6), (48_000, 2)]), None, "surround");
        assert_eq!(mix_format(&[(0, 1), (48_000, 2)]), None);
        assert!(Mixer::new(48_000, 1, &[plain(48_000, 2)]).is_none());
        assert!(Mixer::new(48_000, 2, &[]).is_none());
        assert!(Mixer::new(0, 2, &[plain(48_000, 2)]).is_none());
    }

    /// A plain sum, sample for sample, of voices at the mix's own rate —
    /// clamped to full scale, never wrapped or scaled down.
    #[test]
    fn voices_are_summed_and_clamped_to_full_scale() {
        let out = mixed(
            48_000,
            1,
            &[
                (plain(48_000, 1), vec![vec![0.25, 0.5, 0.8, -0.8, -0.1]]),
                (plain(48_000, 1), vec![vec![0.25, -0.25, 0.8, -0.8]]),
            ],
        );
        assert_eq!(out, vec![vec![0.5, 0.25, 1.0, -1.0, -0.1]]);
    }

    /// A mono voice sounds in both channels of a stereo mix; a stereo one
    /// keeps its left and right.
    #[test]
    fn a_mono_voice_is_heard_in_both_channels_of_a_stereo_mix() {
        let out = mixed(
            48_000,
            2,
            &[
                (plain(48_000, 2), vec![vec![0.1, 0.1], vec![-0.1, -0.1]]),
                (plain(48_000, 1), vec![vec![0.5, 0.25]]),
            ],
        );
        assert_eq!(out, vec![vec![0.6, 0.35], vec![0.4, 0.15]]);
    }

    /// Each voice at its own place in time: a lead is silence in front of
    /// it; priming is cut from its front, and padding past what it presents
    /// from its end; the mix lasts until the last voice ends.
    #[test]
    fn each_voice_sounds_at_its_own_place_in_time() {
        let early = Voice {
            skip: 2,
            length: Some(3),
            ..plain(48_000, 1)
        };
        let late = Voice {
            lead: 4,
            ..plain(48_000, 1)
        };
        let out = mixed(
            48_000,
            1,
            &[
                // Two of priming, three presented, two of padding.
                (early, vec![vec![9.0, 9.0, 0.1, 0.2, 0.3, 9.0, 9.0]]),
                (late, vec![vec![0.5, 0.5]]),
            ],
        );
        assert_eq!(out, vec![vec![0.1, 0.2, 0.3, 0.0, 0.5, 0.5]]);
    }

    /// Another rate is brought to the mix's by linear interpolation: exact
    /// halfway for half the rate, and a ramp stays a ramp at any ratio.
    #[test]
    fn a_voice_at_another_rate_is_resampled_to_the_mixs() {
        let out = mixed(
            48_000,
            1,
            &[(plain(24_000, 1), vec![vec![0.0, 0.5, 1.0, 0.5]])],
        );
        assert_eq!(out, vec![vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25]]);

        // 44.1 kHz under 48 kHz: frame j lands at j·44100/48000.
        let ramp: Vec<f32> = (0..4410).map(|i| i as f32 / 8192.0).collect();
        let out = mixed(48_000, 1, &[(plain(44_100, 1), vec![ramp])]);
        assert_eq!(out[0].len(), 4800);
        for (j, &sample) in out[0].iter().enumerate().take(4790) {
            let expected = (j as f64 * 44_100.0 / 48_000.0) / 8192.0;
            assert!(
                (f64::from(sample) - expected).abs() < 1e-5,
                "{j}: {sample} against {expected}"
            );
        }
        // A voice's lead is in MIX frames whatever its own rate; its
        // priming is in its own.
        let voice = Voice {
            lead: 2,
            skip: 1,
            ..plain(24_000, 1)
        };
        let out = mixed(48_000, 1, &[(voice, vec![vec![9.0, 0.0, 1.0]])]);
        assert_eq!(out, vec![vec![0.0, 0.0, 0.0, 0.5, 1.0, 0.5]]);
    }

    /// The same sound in any pieces, in any order of arrival between the
    /// voices, makes the same mix to the bit — and none of it is made until
    /// every voice has handed in what it needs.
    #[test]
    fn the_mix_is_the_same_whatever_pieces_the_sound_arrives_in() {
        let a: Vec<f32> = (0..5000)
            .map(|i| ((i * 7919) % 2000) as f32 / 2000.0 - 0.5)
            .collect();
        let b: Vec<f32> = (0..3000)
            .map(|i| ((i * 104_729) % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        let c: Vec<f32> = b.iter().map(|x| -x * 0.5).collect();
        let voices = [
            Voice {
                skip: 100,
                ..plain(48_000, 1)
            },
            Voice {
                lead: 700,
                length: Some(2500),
                ..plain(44_100, 2)
            },
        ];
        let whole = mixed(
            48_000,
            2,
            &[
                (voices[0], vec![a.clone()]),
                (voices[1], vec![b.clone(), c.clone()]),
            ],
        );
        for (first, second) in [(37usize, 501usize), (1024, 1024), (4999, 1)] {
            let mut mixer = Mixer::new(48_000, 2, &voices).unwrap();
            let mut out = vec![Vec::new(), Vec::new()];
            let (mut at_a, mut at_b) = (0usize, 0usize);
            assert_eq!(mixer.ready(), 0, "nothing until every voice has spoken");
            while at_a < a.len() || at_b < b.len() {
                if at_a < a.len() {
                    let end = (at_a + first).min(a.len());
                    assert!(mixer.push(0, &[&a[at_a..end]]));
                    at_a = end;
                }
                if at_b < b.len() {
                    let end = (at_b + second).min(b.len());
                    assert!(mixer.push(1, &[&b[at_b..end], &c[at_b..end]]));
                    at_b = end;
                }
                let block = mixer.take(333);
                for (all, part) in out.iter_mut().zip(block) {
                    all.extend(part);
                }
            }
            mixer.finish(0);
            mixer.finish(1);
            for (all, part) in out.iter_mut().zip(drain(&mut mixer, 333)) {
                all.extend(part);
            }
            assert_eq!(out, whole, "pieces of {first} and {second}");
        }
    }

    /// A voice that has handed in nothing holds the mix back until it does
    /// — or until it has finished, when it is silence — and what has been
    /// mixed is let go of.
    #[test]
    fn a_silent_voice_holds_the_mix_back_until_it_speaks_or_ends() {
        let mut mixer = Mixer::new(48_000, 1, &[plain(48_000, 1), plain(48_000, 1)]).unwrap();
        assert!(mixer.push(0, &[&[0.5; 100]]));
        assert_eq!(mixer.ready(), 0);
        assert!(mixer.push(1, &[&[0.25; 40]]));
        // Frame i needs i + 1 of each: 39 frames are certain.
        assert_eq!(mixer.ready(), 39);
        assert_eq!(mixer.take(1000), vec![vec![0.75; 39]]);
        assert_eq!(mixer.buffered(1), 1);
        assert_eq!(mixer.buffered(0), 61);
        mixer.finish(1);
        assert_eq!(mixer.ready(), 61 - 1);
        mixer.finish(0);
        assert_eq!(mixer.length(), Some(100));
        let rest = mixer.take(1000);
        assert_eq!(rest[0][0], 0.75);
        assert_eq!(rest[0][1], 0.5);
        assert_eq!(rest[0].len(), 61);
        assert!(mixer.done());
        // A voice whose edit starts past everything handed in so far holds
        // none of it: it is not "ahead", it has not reached its start.
        let late_start = Voice {
            skip: 5_000,
            ..plain(48_000, 1)
        };
        let mut mixer = Mixer::new(48_000, 1, &[late_start, plain(48_000, 1)]).unwrap();
        for _ in 0..5 {
            assert!(mixer.push(0, &[&[9.0; 1_000]]));
            assert_eq!(mixer.buffered(0), 0);
        }
        assert!(mixer.push(0, &[&[0.5; 10]]));
        assert_eq!(mixer.buffered(0), 10);
        assert!(mixer.push(1, &[&[0.25; 10]]));
        assert_eq!(mixer.take(100), vec![vec![0.75; 9]]);
        // Wrong shapes are refused, and change nothing.
        let mut mixer = Mixer::new(48_000, 2, &[plain(48_000, 2)]).unwrap();
        assert!(!mixer.push(0, &[&[0.1; 4]]));
        assert!(!mixer.push(0, &[&[0.1; 4], &[0.1; 3]]));
        assert!(!mixer.push(1, &[&[0.1; 4], &[0.1; 4]]));
        assert_eq!(mixer.buffered(0), 0);
        mixer.finish(0);
        assert!(!mixer.push(0, &[&[0.1; 4], &[0.1; 4]]), "after its end");
        assert!(mixer.done(), "a voice with nothing to say ends at once");
    }

    /// A track's edit list read the way the single-track path reads it:
    /// its lead in the movie's timescale, its priming and presented length
    /// in its own — at the rate its decoder hands out.
    #[test]
    fn a_tracks_edit_places_its_voice() {
        let track = Track {
            id: 2,
            handler: *b"soun",
            enabled: true,
            timescale: 44_100,
            duration: 441_000,
            width: 0,
            height: 0,
            rotation: Some(0),
            entry: None,
            samples: Vec::new(),
            edit: Some(Edit {
                media_start: 2112,
                lead: 300,
                length: Some(5_000),
            }),
            complex_edit: false,
        };
        let movie = Movie {
            timescale: 600,
            duration: 6_000,
            tracks: Vec::new(),
            playing_sound: Vec::new(),
        };
        // HE-AAC: decoded at 88.2 kHz from a 44.1 kHz index.
        let placed = voice(&movie, &track, 88_200, 2, 96_000).unwrap();
        assert_eq!(
            placed,
            Voice {
                rate: 88_200,
                channels: 2,
                // 0.5 s of the movie's 600, at the mix's 96 kHz.
                lead: 48_000,
                skip: 4224,
                // 5000 / 600 s = 367 500 ticks at 44.1 kHz, doubled.
                length: Some(735_000),
            }
        );
        // No edit list: all of it, from the start.
        let bare = Track {
            edit: None,
            ..track.clone()
        };
        assert_eq!(
            voice(&movie, &bare, 44_100, 1, 48_000),
            Some(plain(44_100, 1))
        );
        // An edit this reader does not follow, or nothing to divide by.
        let complex = Track {
            complex_edit: true,
            ..track.clone()
        };
        assert_eq!(voice(&movie, &complex, 44_100, 1, 48_000), None);
        let timeless = Movie {
            timescale: 0,
            ..movie.clone()
        };
        assert_eq!(voice(&timeless, &track, 44_100, 1, 48_000), None);
    }
}
