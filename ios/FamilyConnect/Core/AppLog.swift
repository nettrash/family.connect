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
    static let ui     = Logger(subsystem: subsystem, category: "ui")
}
