//
//  NoteColor.swift
//  FamilyConnect
//
//  The board's palette, shared by both platforms.
//
//  Lives on its own because the Mac board needs it too, and it is a
//  lookup table rather than a view — the only reason it sat inside
//  BoardView is that the phone's board was written first.
//

import SwiftUI

/// The six names the protocol allows, and their colours. An unknown name
/// from a newer server falls back rather than failing — the note still has
/// to be readable.
nonisolated enum NoteColor {
    static let palette = ["yellow", "pink", "blue", "green", "orange", "purple"]

    static func swiftUI(_ name: String) -> Color {
        switch name {
        case "pink": Color(red: 0.99, green: 0.78, blue: 0.85)
        case "blue": Color(red: 0.76, green: 0.88, blue: 0.99)
        case "green": Color(red: 0.79, green: 0.94, blue: 0.79)
        case "orange": Color(red: 1.00, green: 0.85, blue: 0.70)
        case "purple": Color(red: 0.88, green: 0.82, blue: 0.98)
        default: Color(red: 1.00, green: 0.95, blue: 0.70)
        }
    }
}
