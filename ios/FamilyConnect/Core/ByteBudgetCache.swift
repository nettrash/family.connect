//
//  ByteBudgetCache.swift
//  FamilyConnect
//
//  A least-recently-used cache bounded by BYTES, not by entries — what the
//  decoded pictures in front of the attachment cache (`AttachmentStore`)
//  and the decoded frames of a moving sticker (`StickerFrames`) are kept
//  in.
//
//  WHY BYTES. The cache it replaces held forty images, first in first out.
//  Forty is the wrong unit twice over: a 600-pixel preview is about a
//  megabyte decoded and a 1440-pixel photo eight, so "forty" was anything
//  from 40 MB to 320 MB depending on what happened to be in the chat; and
//  a thread window of sixty rows with a few albums in it holds more than
//  forty pictures, so first-in-first-out evicted pictures that were still
//  on screen while the reader scrolled past them. Counting what an entry
//  actually costs makes the bound mean something, and touching an entry on
//  every read keeps the pictures being drawn at the young end.
//
//  WHY NOT NSCache. It was the obvious candidate and is the wrong one for
//  how these entries are produced. Both callers decode OFF the main actor
//  and tell the view by observation when the picture lands, so the view
//  then reads the cache again. NSCache may evict at any moment — under its
//  own cost limit as well as under memory pressure, in an order it does
//  not document — and an entry dropped between "inserted" and "read back"
//  is a view that asks again, a decode that runs again, and an insert that
//  can be dropped again. Here the newest entry ALWAYS survives its own
//  insert (even one larger than the whole budget, which then stands
//  alone), eviction is strictly oldest-first, and the whole thing is
//  deterministic enough to test. Memory pressure is still answered — the
//  owner calls `removeAll()` on the system's warning, which only costs a
//  re-decode of what is on screen.
//
//  Not thread-safe, on purpose: both owners are main-actor objects that
//  only ever touch it from there.
//

import Foundation

nonisolated final class ByteBudgetCache<Key: Hashable, Value> {

    /// One entry, in a doubly linked list from oldest to newest. A class so
    /// that moving it to the young end is two pointer swaps, not a search.
    private final class Node {
        let key: Key
        var value: Value
        var cost: Int
        var older: Node?
        var newer: Node?

        init(key: Key, value: Value, cost: Int) {
            self.key = key
            self.value = value
            self.cost = cost
        }
    }

    /// The most the entries may cost together. The newest entry is kept
    /// even when it alone is over it (see the header).
    let budget: Int
    private(set) var totalCost = 0
    private var nodes: [Key: Node] = [:]
    private var oldest: Node?
    private var newest: Node?

    init(budget: Int) {
        self.budget = max(budget, 0)
    }

    var count: Int { nodes.count }

    /// Whether the key is held, WITHOUT counting as a use — for callers
    /// deciding whether to start work, which is not the same as drawing.
    func contains(_ key: Key) -> Bool {
        nodes[key] != nil
    }

    /// The value, if held, made the most recently used.
    func value(for key: Key) -> Value? {
        guard let node = nodes[key] else { return nil }
        moveToNewest(node)
        return node.value
    }

    /// Hold `value` under `key` at `cost` bytes, replacing what was there,
    /// then evict from the old end until the budget holds again — never the
    /// entry just inserted.
    func insert(_ value: Value, for key: Key, cost: Int) {
        let cost = max(cost, 0)
        if let node = nodes[key] {
            totalCost += cost - node.cost
            node.value = value
            node.cost = cost
            moveToNewest(node)
        } else {
            let node = Node(key: key, value: value, cost: cost)
            nodes[key] = node
            totalCost += cost
            append(node)
        }
        while totalCost > budget, let victim = oldest, victim !== newest {
            remove(node: victim)
        }
    }

    /// Drop one entry. A key that is not held is a no-op.
    func remove(_ key: Key) {
        guard let node = nodes[key] else { return }
        remove(node: node)
    }

    /// Drop every entry whose key matches — the targeted forget, where the
    /// key carries more than the id being forgotten (a pixel size, say).
    func removeAll(where shouldRemove: (Key) -> Bool) {
        for key in nodes.keys where shouldRemove(key) {
            remove(key)
        }
    }

    func removeAll() {
        // Break the links explicitly: the list holds strong references in
        // both directions, and dropping only the dictionary would leave
        // every node keeping its neighbours alive.
        var node = oldest
        while let current = node {
            node = current.newer
            current.older = nil
            current.newer = nil
        }
        nodes.removeAll()
        oldest = nil
        newest = nil
        totalCost = 0
    }

    /// The keys from least to most recently used — test-facing.
    var keysOldestFirst: [Key] {
        var keys: [Key] = []
        var node = oldest
        while let current = node {
            keys.append(current.key)
            node = current.newer
        }
        return keys
    }

    // MARK: - The list

    private func append(_ node: Node) {
        node.older = newest
        node.newer = nil
        newest?.newer = node
        newest = node
        if oldest == nil { oldest = node }
    }

    private func unlink(_ node: Node) {
        if let older = node.older { older.newer = node.newer } else { oldest = node.newer }
        if let newer = node.newer { newer.older = node.older } else { newest = node.older }
        node.older = nil
        node.newer = nil
    }

    private func moveToNewest(_ node: Node) {
        guard node !== newest else { return }
        unlink(node)
        append(node)
    }

    private func remove(node: Node) {
        unlink(node)
        nodes[node.key] = nil
        totalCost -= node.cost
    }
}
