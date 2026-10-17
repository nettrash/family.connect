//
//  NewLineKeys.swift
//  FamilyConnect
//
//  Which Return makes a new line (issue #95). Return on its own sends —
//  the slot's own rule (MacRecordSlot.takesReturn) — and Return held with
//  ANY of the modifiers people reach for, ⌥ ⌘ ⌃ or ⇧, is a new line in the
//  field. No setting: everybody's habit works, whichever app taught it.
//
//  The same answer on every desktop client: Windows' NewLineKeys (Shift,
//  Ctrl, Alt) and the web's `composer::return_makes_line`. Pure, so both
//  test runs pin it; MacNewLineKeys is the Mac's wiring around it.
//

import Foundation

nonisolated enum NewLineKeys {

    /// Return pressed with these modifiers held: a new line, or not (then it
    /// is the field's ordinary Return — Send in a composer).
    static func returnMakesLine(shift: Bool, option: Bool, command: Bool, control: Bool) -> Bool {
        shift || option || command || control
    }
}
