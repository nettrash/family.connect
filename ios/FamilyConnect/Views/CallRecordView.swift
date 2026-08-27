//
//  CallRecordView.swift
//  FamilyConnect
//
//  The bubble content of a call record (docs/protocol.md, "Voice calls"):
//  a phone glyph and one line of wording, drawn INSTEAD of the body —
//  which is an English placeholder for clients that predate calls. Shared
//  by the phone and the Mac, like the poll and the location.
//
//  Tapping it calls back. The double-tap and long-press are re-emitted to
//  the enclosing bubble, the way every child block does, so the quick
//  heart and the context menu keep working over it.
//

import SwiftUI

struct CallRecordView: View {
    let call: CallDTO
    let isMine: Bool
    /// Tap: ring the peer again. nil when there is nobody to call back
    /// (calls are off, or this is not a direct chat).
    var onCallBack: (() -> Void)?
    var onDoubleTap: () -> Void = {}
    var onLongPress: () -> Void = {}

    private var glyph: String {
        // A video record gets the camera glyph family; SF has no
        // video twin of phone.arrow.up.right, so direction is carried by
        // the wording (and the missed tint) rather than the symbol.
        if call.video {
            switch call.outcome {
            case CallDTO.Outcome.declined, CallDTO.Outcome.failed:
                return "video.slash"
            default:
                return "video.fill"
            }
        }
        switch call.outcome {
        case CallDTO.Outcome.missed:
            return isMine ? "phone.arrow.up.right" : "phone.arrow.down.left"
        case CallDTO.Outcome.declined:
            return "phone.down"
        case CallDTO.Outcome.failed:
            return "phone.badge.waveform"
        default:
            return isMine ? "phone.arrow.up.right" : "phone.arrow.down.left"
        }
    }

    private var isMissed: Bool {
        call.outcome == CallDTO.Outcome.missed && !isMine
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: glyph)
                .font(.title3)
                .foregroundStyle(isMissed ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(CallRecordText.label(call, isMine: isMine))
                    .font(.body)
                    .fixedSize(horizontal: false, vertical: true)
                if onCallBack != nil {
                    Text("Call back")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { onDoubleTap() }
        .onTapGesture { onCallBack?() }
        .simultaneousGesture(LongPressGesture().onEnded { _ in onLongPress() })
        .accessibilityElement(children: .combine)
        .accessibilityLabel(CallRecordText.label(call, isMine: isMine))
        .accessibilityAddTraits(onCallBack != nil ? .isButton : [])
        // The visible "Call back" caption is folded away by .combine; say
        // what activating does, gated exactly like the trait above. The
        // outcome stays the label — the hint must not lead.
        .accessibilityHint(onCallBack != nil ? Text("Calls back") : Text(verbatim: ""))
    }
}
