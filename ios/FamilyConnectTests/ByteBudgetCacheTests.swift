//
//  ByteBudgetCacheTests.swift
//  FamilyConnectTests
//
//  The least-recently-used, byte-bounded cache the decoded pictures and the
//  moving stickers' frames are kept in. The properties the callers lean on,
//  each pinned: eviction is strictly oldest-USED first, the budget is in
//  bytes, and the entry just inserted always survives its own insert — the
//  one guarantee that keeps "decode off the main actor, then wake the view
//  to read the cache" from looping.
//

import Testing
@testable import FamilyConnect

@Suite("Byte-budget cache")
struct ByteBudgetCacheTests {

    @Test("evicts the least recently USED entry once the bytes run over")
    func evictsLeastRecentlyUsed() {
        let cache = ByteBudgetCache<String, Int>(budget: 30)
        cache.insert(1, for: "a", cost: 10)
        cache.insert(2, for: "b", cost: 10)
        cache.insert(3, for: "c", cost: 10)
        // Reading "a" makes it the youngest, so "b" is now the oldest.
        #expect(cache.value(for: "a") == 1)
        cache.insert(4, for: "d", cost: 10)
        #expect(!cache.contains("b"))
        #expect(cache.keysOldestFirst == ["c", "a", "d"])
        #expect(cache.totalCost == 30)
    }

    @Test("the budget is bytes, not entries")
    func budgetIsBytes() {
        let cache = ByteBudgetCache<String, Int>(budget: 100)
        for index in 0..<50 { cache.insert(index, for: "\(index)", cost: 1) }
        #expect(cache.count == 50)
        // One large entry pushes out as many small ones as it needs to.
        cache.insert(99, for: "big", cost: 80)
        #expect(cache.totalCost <= 100)
        #expect(cache.count == 21)
        #expect(cache.contains("big"))
        #expect(!cache.contains("0"))
        #expect(cache.contains("49"))
    }

    @Test("the newest entry survives its own insert, even over the whole budget")
    func newestAlwaysSurvives() {
        let cache = ByteBudgetCache<String, Int>(budget: 10)
        cache.insert(1, for: "a", cost: 5)
        cache.insert(2, for: "huge", cost: 50)
        #expect(cache.contains("huge"))
        #expect(!cache.contains("a"))
        #expect(cache.count == 1)
        // And the next insert evicts it like anything else.
        cache.insert(3, for: "b", cost: 5)
        #expect(!cache.contains("huge"))
        #expect(cache.totalCost == 5)
    }

    @Test("replacing an entry re-charges it and makes it the youngest")
    func replaceRecharges() {
        let cache = ByteBudgetCache<String, Int>(budget: 100)
        cache.insert(1, for: "a", cost: 10)
        cache.insert(2, for: "b", cost: 10)
        cache.insert(3, for: "a", cost: 40)
        #expect(cache.totalCost == 50)
        #expect(cache.keysOldestFirst == ["b", "a"])
        #expect(cache.value(for: "a") == 3)
    }

    @Test("contains is not a use; value(for:) is")
    func containsDoesNotTouch() {
        let cache = ByteBudgetCache<String, Int>(budget: 20)
        cache.insert(1, for: "a", cost: 10)
        cache.insert(2, for: "b", cost: 10)
        #expect(cache.contains("a"))
        cache.insert(3, for: "c", cost: 10)
        #expect(!cache.contains("a"), "a lookup that only asks must not keep an entry alive")
    }

    @Test("targeted and total removal keep the books straight")
    func removal() {
        let cache = ByteBudgetCache<String, Int>(budget: 100)
        cache.insert(1, for: "7@128", cost: 10)
        cache.insert(2, for: "7@256", cost: 20)
        cache.insert(3, for: "8@128", cost: 30)
        cache.removeAll { $0.hasPrefix("7@") }
        #expect(cache.keysOldestFirst == ["8@128"])
        #expect(cache.totalCost == 30)
        cache.remove("8@128")
        cache.remove("absent")
        #expect(cache.count == 0)
        #expect(cache.totalCost == 0)
        cache.insert(4, for: "x", cost: 1)
        cache.removeAll()
        #expect(cache.count == 0)
        #expect(cache.totalCost == 0)
        #expect(cache.keysOldestFirst.isEmpty)
    }
}
