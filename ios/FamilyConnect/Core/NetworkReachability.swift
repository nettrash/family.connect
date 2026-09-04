//
//  NetworkReachability.swift
//  FamilyConnect
//
//  "The network came back" — the one signal this app did not have.
//
//  WHY THIS EXISTS. Everything that recovers from a bad network here was
//  driven by the socket's own reconnect loop: it sleeps a full-jitter
//  backoff, dials, and on success the coordinator resyncs and flushes the
//  outbox. That is right for a server that went away and wrong for a phone
//  that has just come out of a tunnel — the route is back NOW, and the app
//  would otherwise sit at "Connecting…" for up to the 30 s ceiling with
//  messages queued behind it, having learned nothing from the fact that
//  the radio just came up. Android has had `ConnectivityObserver` from the
//  beginning and cuts its backoff short; this is the missing half of that
//  parity (docs/protocol.md, "Sending on an unreliable network": a
//  returning network is a trigger).
//
//  WHAT IT DELIBERATELY DOES NOT DO: gate anything. Nothing in this app
//  asks "are we online?" before trying — a send attempt on a dead radio
//  fails in milliseconds and costs nothing, while a reachability check
//  used as a gate is a well-known way to refuse work on a network that
//  would have carried it (captive portals, VPNs, `satisfied` on an
//  interface with no route). This type only ever says "try again now".
//
//  DEBOUNCED, because path updates arrive in bursts: switching from
//  cellular to Wi-Fi can fire several satisfied paths in a second, and an
//  undebounced kick would turn that into its own little reconnect storm.
//
//  Android counterpart: data/net/ConnectivityObserver.kt.
//

import Foundation
import Network
import os

@MainActor
final class NetworkReachability {

    /// Called when a route appears after there was none — and only then.
    /// Never called for a path that was already satisfied.
    var onRestored: () -> Void = {}

    /// Last known answer. Read for display only; never as a gate.
    private(set) var isOnline = true

    private let monitor: NWPathMonitor
    private let queue = DispatchQueue(label: "me.nettrash.FamilyConnect.reachability")
    private var started = false
    private var lastRestoredAt: Date?

    /// Two path updates closer together than this count as one event.
    private let debounce: TimeInterval

    init(monitor: NWPathMonitor = NWPathMonitor(), debounce: TimeInterval = 2) {
        self.monitor = monitor
        self.debounce = debounce
    }

    func start() {
        guard !started else { return }
        started = true
        monitor.pathUpdateHandler = { [weak self] path in
            let satisfied = path.status == .satisfied
            Task { @MainActor [weak self] in
                self?.apply(satisfied: satisfied)
            }
        }
        monitor.start(queue: queue)
    }

    func stop() {
        guard started else { return }
        started = false
        monitor.pathUpdateHandler = nil
        monitor.cancel()
    }

    /// Test seam: drive the transition without a network.
    func apply(satisfied: Bool, now: Date = Date()) {
        let wasOnline = isOnline
        isOnline = satisfied
        guard satisfied, !wasOnline else { return }
        if let last = lastRestoredAt, now.timeIntervalSince(last) < debounce { return }
        lastRestoredAt = now
        AppLog.sync.info("Network restored")
        onRestored()
    }
}
