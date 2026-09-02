//
//  LocationAuthorizationCompat.swift
//  FamilyConnectTests
//
//  One constant, because `CLAuthorizationStatus.authorizedWhenInUse` is
//  `@available(macOS, unavailable)` and the location tests need a granted
//  status they can ASSIGN.
//
//  Swift treats the two positions differently, which is why this was
//  invisible until someone pointed a Mac destination at the test target:
//  an unavailable enum case may still appear in a PATTERN (the compiler
//  allows it so a `switch` stays exhaustive), so LocationProvider's own
//  `case .authorizedAlways, .authorizedWhenInUse:` builds for macOS
//  perfectly well. Writing the same case as a VALUE — `= .authorizedWhenInUse`,
//  or passing it as an argument — is a hard error there. Every one of the
//  six breakages was in a test, which is exactly why the app compiled for
//  the Mac while `xcodebuild test -destination 'platform=macOS'` did not.
//
//  `.authorizedAlways` is the right stand-in rather than a lie for the
//  Mac's benefit: LocationProvider treats the two identically at all four
//  of its decision points (requestPermission, currentFix,
//  authorizationDidChange, and the freshness path), so a test asserting
//  "permission is granted" asserts the same thing with either. On iOS the
//  constant still resolves to the case the phone actually reports, so the
//  iOS run keeps testing the real value.
//

import CoreLocation

/// A granted authorization status, spelled the way the current platform
/// can spell it. Use this anywhere a test needs to *set* "allowed".
let grantedLocationAuthorization: CLAuthorizationStatus = {
    #if os(macOS)
    return .authorizedAlways
    #else
    return .authorizedWhenInUse
    #endif
}()
