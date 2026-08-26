//
//  SocketHold.swift
//  FamilyConnect
//
//  Whether backgrounding suspends the socket. Ordinarily it does — iOS
//  would kill it anyway, and the server pushes to a device with no socket.
//  A call changes that: its `call_end`, its ICE candidates and an ICE
//  restart all arrive over the socket, and the platform permissions a
//  call runs under (an active audio session, the VoIP background mode)
//  are exactly what let a socket stay up (docs/protocol.md, "Voice
//  calls"). Written as arithmetic, the ThreadFollow idiom, so the rule is
//  testable without a socket or a scene.
//

import Foundation

nonisolated enum SocketHold {

    enum Decision: Equatable, Sendable {
        /// Drop the socket; REST resync brings everything back.
        case suspend
        /// Keep it: a call is in progress and needs it.
        case keep
    }

    /// `isInBackground` is where the scene is going; `isCallInProgress`
    /// is whether the call machine is anywhere but idle — ringing counts,
    /// on both ends, because the answer has to arrive.
    static func decide(isInBackground: Bool, isCallInProgress: Bool) -> Decision {
        guard isInBackground else { return .keep }
        return isCallInProgress ? .keep : .suspend
    }
}
