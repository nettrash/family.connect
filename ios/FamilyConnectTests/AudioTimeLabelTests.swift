//
//  AudioTimeLabelTests.swift
//  FamilyConnectTests
//
//  `AudioRecorder.timeLabel` is four lines and three screens depend on it:
//  the counter ticking up in each composer while a voice note records, and
//  both ends of an audio bubble's scrubber (AudioPlayerView) — where it is
//  also the VoiceOver label, "Audio, 0:07". It had no test (issue #33).
//
//  Pure formatting, so what is pinned is the shape and the edges: the
//  seconds are always two digits, the rounding is away from zero, and the
//  boundary that matters is the one where a rounded 59.5 has to carry into
//  1:00 rather than print 0:60.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Audio time label")
@MainActor
struct AudioTimeLabelTests {

    @Test("Zero, and anything below zero, reads 0:00")
    func zeroFloor() {
        #expect(AudioRecorder.timeLabel(0) == "0:00")
        // The recorder resets `elapsed` to 0 before it starts, but the
        // player's is driven by a time observer: a clamp is cheaper than
        // trusting AVFoundation never to hand back a negative.
        #expect(AudioRecorder.timeLabel(-1) == "0:00")
        #expect(AudioRecorder.timeLabel(-0.4) == "0:00")
        #expect(AudioRecorder.timeLabel(-999) == "0:00")
    }

    @Test("Under a minute keeps a leading 0 and two second digits")
    func underOneMinute() {
        #expect(AudioRecorder.timeLabel(1) == "0:01")
        #expect(AudioRecorder.timeLabel(7) == "0:07")
        #expect(AudioRecorder.timeLabel(9) == "0:09")
        #expect(AudioRecorder.timeLabel(10) == "0:10")
        #expect(AudioRecorder.timeLabel(59) == "0:59")
    }

    /// Rounding is to nearest, away from zero — a half second counts as a
    /// second, in both directions.
    @Test("Fractional seconds round to the nearest, not toward zero")
    func rounding() {
        #expect(AudioRecorder.timeLabel(0.4) == "0:00")
        #expect(AudioRecorder.timeLabel(0.5) == "0:01")
        #expect(AudioRecorder.timeLabel(0.6) == "0:01")
        #expect(AudioRecorder.timeLabel(1.49) == "0:01")
        #expect(AudioRecorder.timeLabel(1.5) == "0:02")
    }

    /// The boundary worth having a test for: the rounding happens BEFORE
    /// the divide, so 59.5 becomes 60 seconds and therefore 1:00. Rounding
    /// afterwards would have printed "0:60".
    @Test("A rounded 59.5 carries into the minute rather than printing 0:60")
    func roundingCarriesIntoTheMinute() {
        #expect(AudioRecorder.timeLabel(59.4) == "0:59")
        #expect(AudioRecorder.timeLabel(59.5) == "1:00")
        #expect(AudioRecorder.timeLabel(60) == "1:00")
        #expect(AudioRecorder.timeLabel(119.5) == "2:00")
    }

    @Test("A minute and over counts minutes, seconds still two digits")
    func overOneMinute() {
        #expect(AudioRecorder.timeLabel(61) == "1:01")
        #expect(AudioRecorder.timeLabel(65) == "1:05")
        #expect(AudioRecorder.timeLabel(90) == "1:30")
        #expect(AudioRecorder.timeLabel(599) == "9:59")
    }

    /// Two-digit minutes are NOT padded — "10:00", not "010:00" — which is
    /// why the label is `%d:%02d` rather than `%02d:%02d`.
    @Test("Ten minutes and over widens the minutes without padding them")
    func tenMinutesAndOver() {
        #expect(AudioRecorder.timeLabel(600) == "10:00")
        #expect(AudioRecorder.timeLabel(659) == "10:59")
        #expect(AudioRecorder.timeLabel(1800) == "30:00")
    }

    /// The composer's ceiling, so the longest label a RECORDING can ever
    /// produce is this one — three characters, no hour to worry about.
    @Test("The recorder's own cap is 5:00")
    func theRecorderCaps() {
        #expect(AudioRecorder.maxDuration == 300)
        #expect(AudioRecorder.timeLabel(AudioRecorder.maxDuration) == "5:00")
    }

    /// An hour does not exist in this format: minutes keep counting.
    ///
    /// Unreachable from the recorder (it stops at 5:00) but reachable in a
    /// BUBBLE, where the total comes from the attachment's `durationMS`
    /// off the wire — another client, or a server that says anything it
    /// likes. "61:01" is at least unambiguous and monotonic, which a
    /// truncated "1:01" would not be, so this pins it rather than asking
    /// for an h:mm:ss branch nothing in this app can produce.
    @Test("Past an hour the minutes keep counting instead of rolling over")
    func noHourComponent() {
        #expect(AudioRecorder.timeLabel(3600) == "60:00")
        #expect(AudioRecorder.timeLabel(3661) == "61:01")
    }
}
