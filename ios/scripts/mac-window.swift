// mac-window.swift — list an application's on-screen windows.
//
//   swift ios/scripts/mac-window.swift <owner name> [expected title substring]
//
// Prints one line per window: "<id>\t<width>x<height>\t<title>".
// With an expected title, prints ONLY matching windows, so a capture
// script can pipe it straight into `screencapture -l` and get nothing
// when the wrong family is on screen.
//
// The owner name is the bundle's DISPLAY name. For this app that is
// "Family", NOT "FamilyConnect" — filtering on the product name returns
// zero rows and looks exactly like "the app is not running".

import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write(Data("usage: mac-window.swift <owner> [title]\n".utf8))
    exit(2)
}
let owner = args[1]
let expected = args.count > 2 ? args[2] : nil

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    exit(1)
}
var rows: [(Int, CGFloat, CGFloat, String)] = []
for entry in list {
    guard let name = entry[kCGWindowOwnerName as String] as? String, name == owner,
          let id = entry[kCGWindowNumber as String] as? Int,
          let bounds = entry[kCGWindowBounds as String] as? [String: Any],
          let w = bounds["Width"] as? CGFloat, let h = bounds["Height"] as? CGFloat
    else { continue }
    let title = entry[kCGWindowName as String] as? String ?? ""
    if let expected, !title.contains(expected) { continue }
    // Skip the tiny helper windows AppKit keeps around.
    if w < 200 || h < 200 { continue }
    rows.append((id, w, h, title))
}
// Largest first: the main window is the one worth photographing.
for (id, w, h, title) in rows.sorted(by: { $0.1 * $0.2 > $1.1 * $1.2 }) {
    print("\(id)\t\(Int(w))x\(Int(h))\t\(title)")
}
