// mac-window.swift — list an application's on-screen windows.
//
//   swift ios/scripts/mac-window.swift <owner name> [expected title substring]
//   swift ios/scripts/mac-window.swift --pid <pid> [expected title substring]
//
// Prints one line per window: "<id>\t<width>x<height>\t<title>".
// With an expected title, prints ONLY matching windows, so a capture
// script can pipe it straight into `screencapture -l` and get nothing
// when the wrong family is on screen.
//
// The owner name is the bundle's DISPLAY name. For this app that is
// "Family", NOT "FamilyConnect" — filtering on the product name returns
// zero rows and looks exactly like "the app is not running".
//
// --pid IS THE STRONGER GUARD, and the one the capture script uses. The
// real app and the throwaway screenshot build share the display name
// "Family", so an owner-name match can select a window belonging to
// somebody's actual family. A pid match cannot: the caller passes the pid
// of the build it launched itself. That also frees the capture to follow
// the app into Board, Family and Settings, whose windows are not titled
// after the seeded family and which a title guard would refuse.

import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write(Data("usage: mac-window.swift <owner> [title]\n".utf8))
    exit(2)
}
var owner: String? = nil
var expected: String? = nil
var wantedPID: Int? = nil
if args[1] == "--pid" {
    guard args.count > 2, let pid = Int(args[2]) else {
        FileHandle.standardError.write(Data("--pid needs a number\n".utf8))
        exit(2)
    }
    wantedPID = pid
    expected = args.count > 3 ? args[3] : nil
} else {
    owner = args[1]
    expected = args.count > 2 ? args[2] : nil
}

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    exit(1)
}
var rows: [(Int, CGFloat, CGFloat, String)] = []
for entry in list {
    guard let name = entry[kCGWindowOwnerName as String] as? String,
          let id = entry[kCGWindowNumber as String] as? Int,
          let bounds = entry[kCGWindowBounds as String] as? [String: Any],
          let w = bounds["Width"] as? CGFloat, let h = bounds["Height"] as? CGFloat
    else { continue }
    if let wantedPID {
        guard entry[kCGWindowOwnerPID as String] as? Int == wantedPID else { continue }
    } else if let owner {
        guard name == owner else { continue }
    }
    let title = entry[kCGWindowName as String] as? String ?? ""
    if let expected, !title.contains(expected) { continue }
    // Skip the tiny helper windows AppKit keeps around.
    if w < 200 || h < 200 { continue }
    rows.append((id, w, h, title))
}
// FRONT-MOST FIRST, which is the order CGWindowList already returns with
// .optionOnScreenOnly — deliberately not sorted by area. The window worth
// photographing is the one just opened, and several of this app's screens
// (Family, Settings) are SMALLER than the chat window behind them, so an
// area sort silently photographs the chat instead of the sheet.
for (id, w, h, title) in rows {
    print("\(id)\t\(Int(w))x\(Int(h))\t\(title)")
}
