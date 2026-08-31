//
//  StoreResetTests.swift
//  FamilyConnectTests
//
//  A store that will not open used to be a permanent dead end: every launch
//  drew the same error view, whose only advice was to reinstall — which on
//  macOS does not even clear the store, because the app is sandboxed and its
//  container outlives the app. The app now deletes the store and rebuilds it
//  once before showing that view at all.
//
//  The naming is the part worth pinning. SQLite writes its sidecars as
//  `default.store-wal` and `default.store-shm` — the suffix appended to the
//  WHOLE filename — and an implementation that reaches for
//  `appendingPathExtension` instead produces `default.store.wal`, deletes
//  nothing, and leaves a write-ahead log that can reproduce the very failure
//  the reset was meant to clear. That mistake is invisible: the retry still
//  "runs", it just cannot work.
//

import Foundation
import Testing
@testable import FamilyConnect

struct StoreResetTests {

    @Test("the sidecars are named the way SQLite writes them")
    func sidecarNaming() {
        let store = URL(fileURLWithPath: "/tmp/somewhere/default.store")
        let files = FamilyConnectApp.storeFiles(for: store).map(\.lastPathComponent)

        #expect(files == ["default.store", "default.store-wal", "default.store-shm"],
                "a sidecar was named with a dot instead of a hyphen, so the reset deletes nothing")
    }

    @Test("every file stays in the store's own directory")
    func staysInPlace() {
        let store = URL(fileURLWithPath: "/tmp/some dir/default.store")
        for file in FamilyConnectApp.storeFiles(for: store) {
            #expect(file.deletingLastPathComponent().path == "/tmp/some dir")
        }
    }

    /// The reset must remove all three, and must not object to sidecars that
    /// were never written — a store closed cleanly has no -wal.
    @Test("deleting removes the store and any sidecars, and tolerates missing ones")
    func deleteRemovesWhatIsThere() throws {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("store-reset-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let store = directory.appendingPathComponent("default.store")
        try Data([1]).write(to: store)
        try Data([2]).write(to: directory.appendingPathComponent("default.store-wal"))
        // No -shm on purpose.

        FamilyConnectApp.deleteStore(at: store)

        let left = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        #expect(left.isEmpty, "the reset left \(left) behind")
    }

    @Test("the macOS advice does not tell people to reinstall")
    func macAdviceIsNotReinstall() {
        // The wrong advice was the whole of the macOS half of this issue:
        // deleting a sandboxed Mac app leaves ~/Library/Containers intact.
        #if os(macOS)
        #expect(StoreErrorAdvice.text.contains("~/Library/Containers"))
        #expect(!StoreErrorAdvice.text.lowercased().contains("reinstall"))
        #else
        #expect(StoreErrorAdvice.text.lowercased().contains("reinstall"))
        #endif
    }
}
