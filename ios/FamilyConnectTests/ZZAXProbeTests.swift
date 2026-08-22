//
//  ZZAXProbeTests.swift  (TEMPORARY INVESTIGATION FILE — delete after use)
//

import SwiftUI
import UIKit
import Testing

@MainActor
private final class Box {
    var fired = false
}

private struct TapOnlyProbe: View {
    let box: Box
    var body: some View {
        HStack(spacing: 6) {
            Text("Ann")
            Text("See you at six")
        }
        .padding(4)
        .background(Color.gray.opacity(0.2))
        .contentShape(Rectangle())
        .onTapGesture { box.fired = true }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("PROBE_TAPONLY")
        .accessibilityAddTraits(.isButton)
    }
}

private struct TapPlusActionProbe: View {
    let box: Box
    var body: some View {
        HStack(spacing: 6) {
            Text("Ann")
            Text("See you at six")
        }
        .padding(4)
        .background(Color.gray.opacity(0.2))
        .contentShape(Rectangle())
        .onTapGesture { box.fired = true }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("PROBE_TAPACTION")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { box.fired = true }
    }
}

private struct ButtonProbe: View {
    let box: Box
    var body: some View {
        Button {
            box.fired = true
        } label: {
            Text("Ann See you at six")
        }
        .accessibilityLabel("PROBE_BUTTON")
    }
}

@MainActor
private func collect(_ root: NSObject, depth: Int, seen: inout Set<ObjectIdentifier>, out: inout [(NSObject, Int)]) {
    guard depth < 25 else { return }
    let id = ObjectIdentifier(root)
    if seen.contains(id) { return }
    seen.insert(id)
    out.append((root, depth))
    if let view = root as? UIView {
        for sub in view.subviews {
            collect(sub, depth: depth + 1, seen: &seen, out: &out)
        }
    }
    let count = root.accessibilityElementCount()
    if count != NSNotFound && count > 0 {
        for i in 0..<count {
            if let child = root.accessibilityElement(at: i) as? NSObject {
                collect(child, depth: depth + 1, seen: &seen, out: &out)
            }
        }
    }
    if let elements = root.accessibilityElements as? [NSObject] {
        for child in elements {
            collect(child, depth: depth + 1, seen: &seen, out: &out)
        }
    }
}

/// Force the process to behave as if an assistive client were attached,
/// so SwiftUI actually materialises its accessibility nodes.
private func enableAXAutomation() -> String {
    guard let handle = dlopen("/usr/lib/libAccessibility.dylib", RTLD_NOW) else {
        return "dlopen libAccessibility FAILED"
    }
    var report = ""
    if let sym = dlsym(handle, "_AXSSetAutomationEnabled") {
        typealias Fn = @convention(c) (Bool) -> Void
        unsafeBitCast(sym, to: Fn.self)(true)
        report += "_AXSSetAutomationEnabled(true) called; "
    } else {
        report += "_AXSSetAutomationEnabled missing; "
    }
    if let sym = dlsym(handle, "_AXSAutomationEnabled") {
        typealias Fn = @convention(c) () -> Bool
        report += "automationEnabled=\(unsafeBitCast(sym, to: Fn.self)()); "
    }
    return report
}

@MainActor
private func probe<V: View>(_ view: V, label: String, box: Box) -> String {
    let axNote = enableAXAutomation()
    let host = UIHostingController(rootView: view)
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
    window.rootViewController = host
    window.makeKeyAndVisible()
    host.view.setNeedsLayout()
    host.view.layoutIfNeeded()
    RunLoop.current.run(until: Date().addingTimeInterval(0.3))

    var seen = Set<ObjectIdentifier>()
    var out: [(NSObject, Int)] = []
    collect(window, depth: 0, seen: &seen, out: &out)

    var report = "=== \(label): \(out.count) nodes [\(axNote)] ===\n"
    for (node, depth) in out {
        report += String(repeating: "  ", count: depth) + "NODE \(type(of: node)) isAX=\(node.isAccessibilityElement) count=\(node.accessibilityElementCount()) lbl=\(node.accessibilityLabel ?? "-")\n"
    }
    var target: NSObject?
    for (node, depth) in out {
        let lbl = node.accessibilityLabel ?? ""
        if !lbl.isEmpty {
            report += String(repeating: "  ", count: depth)
                + "\(type(of: node)) label=\(lbl) traits=\(node.accessibilityTraits.rawValue) "
                + "isAXElement=\(node.isAccessibilityElement) "
                + "customActions=\(node.accessibilityCustomActions?.map(\.name) ?? []) "
                + "activationPoint=\(node.accessibilityActivationPoint) frame=\(node.accessibilityFrame)\n"
        }
        if lbl.contains(label) { target = node }
    }
    guard let target else {
        report += "!! target element NOT FOUND\n"
        return report
    }
    let hasButtonTrait = target.accessibilityTraits.contains(.button)
    box.fired = false
    let activated = target.accessibilityActivate()
    report += "TARGET buttonTrait=\(hasButtonTrait) accessibilityActivate()=\(activated) firedAfterActivate=\(box.fired)\n"
    return report
}

@Suite("AX probe")
@MainActor
struct ZZAXProbeTests {

    @Test("tap gesture only")
    func tapOnly() {
        let box = Box()
        Issue.record(Comment(rawValue: probe(TapOnlyProbe(box: box), label: "PROBE_TAPONLY", box: box)))
    }

    @Test("tap gesture plus accessibilityAction")
    func tapPlusAction() {
        let box = Box()
        Issue.record(Comment(rawValue: probe(TapPlusActionProbe(box: box), label: "PROBE_TAPACTION", box: box)))
    }

    @Test("button")
    func button() {
        let box = Box()
        Issue.record(Comment(rawValue: probe(ButtonProbe(box: box), label: "PROBE_BUTTON", box: box)))
    }
}
