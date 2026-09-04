//
//  AppLog.swift
//  FamilyConnect
//
//  Centralised `os.Logger` instances. Use these instead of `print` /
//  `NSLog` so that Console.app and the unified system log can filter
//  and group by subsystem/category. Message bodies are never logged —
//  only ids, counts and status codes.
//

import Foundation
import os

nonisolated enum AppLog {
    /// Bundle id for the app. Subsystem strings always start with this so
    /// all FamilyConnect logs cluster together in Console.app.
    static let subsystem = "me.nettrash.FamilyConnect"

    static let app    = Logger(subsystem: subsystem, category: "app")
    /// REST traffic: endpoint, status, retry decisions. Never bodies.
    static let api    = Logger(subsystem: subsystem, category: "api")
    /// WebSocket lifecycle: connects, drops, backoff, undecodable frames.
    static let socket = Logger(subsystem: subsystem, category: "socket")
    /// Resync passes, pagination loops, outbox sweeps.
    static let sync   = Logger(subsystem: subsystem, category: "sync")
    /// Push lifecycle: authorization, token registration, tap routing.
    /// Device ids and route kinds only — never tokens or payload bodies.
    static let push   = Logger(subsystem: subsystem, category: "push")
    static let ui     = Logger(subsystem: subsystem, category: "ui")
    /// Voice calls: state transitions and the selected ICE pair — the
    /// minimum needed to diagnose a call from `log show` on the device.
    /// Never an address, never SDP.
    static let call   = Logger(subsystem: subsystem, category: "call")

    /// The remote-video handoff (issue #38): a surface entering or
    /// leaving the view tree, the far side's track arriving, and which of
    /// the two got there first. Spread over three files on three threads,
    /// so every line carries ONE marker — `log show --predicate
    /// 'eventMessage CONTAINS "[FcCallVideo]"'` pulls the whole sequence
    /// out after the fact. Once-per-call events only, never per frame;
    /// object identities only, never contents.
    enum CallVideo {
        static let tag = "[FcCallVideo]"

        /// A stable short id — enough to tell the surface that attached
        /// from the surface that detached when two of them overlap.
        static func id(_ target: AnyObject?) -> String {
            guard let target else { return "none" }
            return "@" + String(UInt(bitPattern: ObjectIdentifier(target).hashValue) & 0xFFFF_FFFF, radix: 16)
        }
    }
}
