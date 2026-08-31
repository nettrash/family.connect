# App Store readiness — first release (iOS + macOS)
**Audited 2026-08-28** against `master` @ `989334f`. Verdict: **NOT READY**.
Produced by a 243-agent audit: 3 build/archive agents, 11 parallel audit dimensions, then two
adversarial verifiers (evidence lens + consequence lens) against every claimed blocker and
should-fix. 148 raw findings → 114 verified → 111 survived, 3 refuted. Skeptics won ties: a finding's
final severity is the *lowest* any verifier landed on.
Every item below is tracked as a GitHub issue under the [v1.0 — first App Store release](https://github.com/nettrash/family.connect/milestone/2) milestone.


---
## Status update — 2026-08-30

**All five blockers are closed in code.** What remains is provisioning and paperwork that needs
nettrash's servers, accounts and decisions — no further code change is required to submit. The verdict below is the 2026-08-28 audit as
it stood; everything in it is kept as the record of that date, and this section is the only thing
that supersedes it.

Revised verdict: **NO CODE BLOCKERS LEFT — the remaining work is yours.** Report/Block ships on
all three clients and the server; four policy and support pages are published with in-app links on
every client; three screenshot sets (iPhone 6.9", iPad 13", Play phone) regenerate from scripts;
`appstore.md` is rewritten against the shipped feature set with every character limit measured; and
`PrivacyInfo.xcprivacy` declares what the app actually collects, with the two required-reason
categories it needs to clear upload validation.

Your queue, in the order it blocks things:

1. Provision the two demo accounts and the seeded reviewer family on fc.nettrash.me, and fill the
   five placeholders in `appstore.md`.
2. Read the live `config.toml` and write six values into the review notes instead of assuming the
   defaults: `[ai]`, `[calls] enabled`/`video_enabled`, `retention_days`, `stun_urls`,
   `turn_urls`/`turn_secret`, and `include_message_body`. `[ai]` gates three separate answers.
3. Set `support_contact` — it is the only escalation path the app draws for a report about the
   family owner, and it ships commented out.
4. Deploy nettrash.me, then paste the Support and Privacy Policy URLs into both consoles.
5. File the App Privacy form against the constants listed in `appstore.md`, and answer export
   compliance.
6. Decide the iPad question, the store name, and whether open registration on your box is
   acceptable at launch.

### What shipped for #1
Built against the protocol first (`docs/protocol.md` amended before each change, and committed):

- **Block** — from a message menu AND from a member roster row, on iOS, macOS and Android.
  Deliberately NOT owner-gated on any surface: the person a member needs to block may BE the
  owner. Client-side hiding plus server-side suppression of pushes, calls and the direct chat.
- **Report** — on a message and on a person, four fixed reasons, with a mandatory disclosure
  saying what the owner will see. The person-level report is reachable only from the roster,
  which is why the roster entry point mattered.
- **The owner's inbox** — `GET /families/reports` listed and resolvable on all three platforms,
  drawn even when empty so an owner learns it exists before the first report arrives.
- **`support_contact`** — shown on the report sheet as the escalation path for the case the
  feature is weakest at: a report about the owner never reaches the owner.
- Plus the three family controls the work depended on: a per-family member cap, a `closed` join
  policy, and owner hand-off on leave.

### Verified by adversarial audit, not by assertion
A 6-agent audit was run specifically to falsify the claim that #1 was complete. It found **five
blocks-review defects that had been missed**, all since fixed:

1. `calls.rs::add_candidate` relayed a suppressed call's live ICE candidates to the blocker.
2. `handlers_call::end` fanned `call_end` to both parties — `finish_call` had the correct filter
   and the comment explaining it a few lines away; the client-initiated path never got it.
3. iOS `loadBlocksFromStore()` had zero call sites, so an offline cold start drew blocked content.
4. iOS block/report errors rendered only inside the owner-gated join-requests section, so the
   non-owner who most needs them saw nothing on failure.
5. Android hidden rows armed no link-preview fetch — the third-party-log oracle the protocol
   forbids, closed on iOS and not ported.

Test state at the time of writing: server 201 unit + 290 integration, iOS 648, Android 813, all
four targets building, both string catalogues complete in nine languages.

### Should-fix items closed since

- **#6 — photos stopped loading permanently after one transient error.** `AttachmentStore` now
  settles a key only on a real 404; a thrown transport error leaves it retryable, and recovery is
  driven by the socket reconnect (`ChatSyncCoordinator.handle(.connected)`), mirroring what
  `AttachmentRepository.kt` already did on Android. The issue's own prescription — bump
  `generation` on failure — would have made a tight refetch loop offline, so it was not followed.
  652 iOS tests pass, and the regression test was mutation-checked: the first version passed
  against the bug, because `APIClient.perform` retries a transient GET once internally.
- **#8 — macOS "Save a copy" was a dead button.** The sandbox granted only
  `files.user-selected.read-only` while the viewer wrote through an NSSavePanel, and both
  `FileManager` calls were `try?`, so a refusal looked exactly like a click that never registered.
  The grant is now `user-selected.read-write` (verified in a signed product) and the writes are a
  do/catch that raises an alert; the separate no-bytes path, which killed the same button for a
  different reason, now reports too. Proving the write end to end needs a signed sandboxed run
  under the real bundle id, which shares state with the everyday app — one manual Save settles it.
- **#7 — macOS shipped with no push entitlement.** `FamilyConnect-macOS.entitlements` requested the
  iOS key `aps-environment`; both Mac profiles grant only `com.apple.developer.aps-environment`
  (development on the dev profile, production on the store one), so Xcode dropped it while signing
  without failing the build. Renamed, and confirmed present in a signed product via
  `codesign -d --entitlements`. **Not yet proven end to end:** one real alert push has to be seen
  arriving at a quit Mac before any listing copy claims it, since the server also needs working
  APNs credentials for the macOS platform row.

### What shipped for #2 and #3
Detail is under each blocker below; the short version:

- **#2** — four pages (an Apple pair and a Play pair, written separately because the platforms
  collect different things), store cards on nettrash.me, and a Privacy Policy / Support row inside
  iOS, macOS and Android settings. Android had no such rows at all before this pass.
- **#3** — 18 screenshots: iPhone 6.9" ×6, iPad 13" ×6, Play phone ×6, all from one seeded
  family and all reproducible from a script rather than by hand. The three Android shots committed
  in August were deleted: they violated Play's 2:1 ratio rule and carried an alpha channel, and
  the new capture path checks both before writing a file.

**Everything about #3 is worth re-running rather than trusting.** The set is only as good as the
seed, and the seed's photo album is generated gradients until somebody puts real photographs in
`server/scripts/screenshot-photos/`.

### Still open, and deliberately not done
- The `[registration]` server switch (#1's own text marks it optional).
- `Report.message_id` is decoded on both platforms but no client offers the protocol's "jump to
  the message"; the frozen excerpt is always drawn, which is the part that matters.
- The review notes still need the containment argument and must stop advertising "Registration is
  also fully open" — that is **#4**, not #1.
- Guideline 1.2's "published contact information" leg needs **#2**, and an operator has to
  actually configure `support_contact`.

---
## Verdict
No — neither platform can be submitted today, but the reason is almost entirely paperwork and one missing feature, not the binary. The app itself is in genuinely good shape: both shipping configurations build clean, 615 unit tests pass on iOS and 607 on macOS, both archives sign and pass Xcode's -validate-for-store, the server is live with a working coturn relay, and account deletion is fully implemented despite what your own doc says. What is missing is everything that surrounds the binary: there is no privacy policy or support page anywhere for this app (both are required App Store Connect fields, so the submission literally cannot be completed), no screenshots, no Report/Block anywhere in a messenger that ships against an openly-registerable server you operate, and ios/docs/appstore.md is a stale draft describing a text-only app with literal [DEMO_USER] placeholders in the review notes. iOS is roughly 3-5 working days from submittable, with the Report/Block work the long pole. macOS is further out and differently blocked: it has no listing copy, no Mac screenshots, no macOS release procedure documented anywhere, plus two one-line entitlement bugs (push silently disabled, Save… a dead button) — my recommendation is to hold macOS and ship iOS first.

## What is already release-ready
The list of remaining work is only credible if it says what is done. All of the following was
verified by running it, not by reading it.

- Both shipping configurations build clean with zero errors, and the whole unit suite passes: 615 @Test declarations / 617 runs, 0 failures on iPhone 17 simulator; 607 passing on My Mac. The server is 190 unit tests + 239 integration tests all green against a throwaway PostgreSQL, with `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` both exiting 0.
- Both archives can be produced and both pass Xcode's -validate-for-store, signed end to end, verifying clean under `codesign --verify --deep --strict`. Everything that usually breaks at archive time is right: arm64-only iOS with a 1024 icon in Assets.car, a complete 16-1024 Mac icon ladder plus .icns, the Share Extension embedded and sandboxed on both platforms, WebRTC.framework embedded with only the correct slices and its own privacy manifest, dSYMs present, and the macOS @executable_path/../Frameworks rpath already set (a trap that normally only shows up on a standalone Mac launch).
- Account deletion is implemented properly end to end on both platforms — DeleteAccountView reached from SettingsView.swift:350 and MacSettingsView.swift:122, POST /api/v1/me/delete scrubbing the account, killing sessions/devices/push tokens, deleting direct chats and sweeping storage, with 33KB of tests. Your own doc calling it an open blocker is simply stale.
- Crasher hygiene is exceptional: zero try!, zero fatalError, zero as!, zero force-unwraps across 33,405 lines of app + extension code, every array subscript guarded, and zero TODO/FIXME/print. The Share Extension copies with FileManager.copyItem rather than Data(contentsOf:), never blocks the main thread, and guarantees completeRequest with a defer.
- Localization is complete and real: 9 languages, 437/437 translatable keys translated in every one, no needsReview and no absent entries, plus 10 AppIntentVocabulary.plist files that are correctly present in the built bundle (the ITMS-90626 fix is intact in the product, not just the source).
- The infrastructure is in better shape than most solo releases: fc.nettrash.me answers healthz in 0.099s behind a valid Let's Encrypt cert, PostgreSQL and the backend are not exposed, and a coturn 4.6.1 relay is already deployed with STUN on UDP/TCP 3478 and turns: on 5349 — the single biggest predicted call risk is already mitigated at the infrastructure level.
- APNs is fully and correctly implemented: ES256 provider JWT with a mandatory 45-minute cache, apns-push-type: alert for messages, a properly separate `bundleid.voip` topic with apns-push-type: voip and apns-expiration for calls, and a dead VoIP token clearing only voip_token rather than deleting the device row. Migrations are additive-only and run under an advisory lock at startup.
- The macOS app is a genuinely native AppKit-backed SwiftUI Mac app, not Catalyst and not an iOS app in a window: ~200KB of parallel MacViews, a NavigationSplitView sidebar, five window scenes with real minimum sizes, NSOpenPanel/NSSharingServicePicker, AVPlayerView instead of the crashing SwiftUI VideoPlayer, and all three known workspace traps identified and worked around in comments. Calls work on Mac, and the missing-CallKit path is properly engineered via time-sensitive notifications with Answer/Decline actions.
- First-run UX on the store build is exactly what the review notes describe: FCDefaultServerURL expands correctly in both the simulator build and the archive, the app lands directly on Welcome Back / Log In · Register with the server footer, no permission prompt fires at cold start, a corrupt store shows a recoverable error view, every network call carries a 15s timeout, and every spinner has a terminating defer.
- docs/protocol.md is genuinely in step with the server — all 47 routes documented, nothing documented that does not exist — and the client/server WebSocket contract is pinned by byte-identical golden JSON on both sides.
- Repo hygiene is clean: no secrets in the working tree or in any of the 78 commits, .gitignore correctly covers the local .p8/keystore files, LICENSE and Cargo.toml carry only the handle identity, CODEOWNERS is right, and there is not a single `// Created by` header anywhere.

## Build and archive evidence

### `ios-build-test`
iOS build + test dimension: NO BLOCKERS. Both shipping-relevant configurations compile clean with zero errors, and the entire unit-test suite passes.

(1) Debug build of the FamilyConnect scheme: ** BUILD SUCCEEDED **, exit 0, 0 errors.
(2) Release-nettrash build via the FamilyConnect-nettrash scheme: ** BUILD SUCCEEDED **, exit 0, 0 errors. Confirmed it is the configuration that ships: that scheme's ArchiveAction buildConfiguration = "Release-nettrash" (FamilyConnect-nettrash.xcscheme:108) and its LaunchAction/ProfileAction are also Release-nettrash; only its TestAction and AnalyzeAction are Debug. The built product Info.plist carries FCDefaultServerURL = https://fc.nettrash.me, and the .app embeds both PlugIns/FamilyConnectShareExtension.appex and Frameworks/WebRTC.framework, so the whole shipping payload links.
(3) Unit tests (-only-testing:FamilyConnectTests): ** TEST SUCCEEDED **, exit 0. xcresulttool: 615 tests, 617 test runs (one parameterized test ran 3 times), passedTests 617, failedTests 0, skippedTests 0, expectedFailures 0, result "Passed", on iPhone 17 / iOS 26.5. NO failing tests to name. The suite is Swift Testing (@Test/@Suite), 615 @Test declarations across 50 test files.

FamilyConnectUITests were deliberately NOT run, per instruction: they need a live server and a signed build.

IMPORTANT ENVIRONMENT CORRECTION: the destination given in the task, 'platform=iOS Simulator,name=iPhone 16 Pro', DOES NOT RESOLVE on this machine. xcodebuild expands a bare name to OS:latest = 26.5, and iPhone 16 Pro exists only on runtimes 18.1/18.2/18.4/18.6 here; there is no 26.x iPhone 16 Pro. Both first attempts died with exit 70 "Unable to find a device matching the provided destination specifier". All three commands were re-run against 'platform=iOS Simulator,id=9A611E4C-7F9D-43B1-9A5F-686CD77CB6F5' (iPhone 17, iOS 26.5) and succeeded. Any CI or scripted invocation that hardcodes name=iPhone 16 Pro will fail here.

WARNINGS in the Release-nettrash build: 26 lines matching 'warning:' = 25 compiler emissions at 23 unique source sites, plus 1 tool warning. The Debug and Release-nettrash warning sets are byte-identical (diff of unique file:line:col warnings returns nothing). Grouped by kind:
  - 20 Swift actor-isolation / concurrency warnings (11 "call to main actor-isolated instance method X in a synchronous nonisolated context", 2 "reference to captured var 'self' in concurrently-executing code", 2 "main actor-isolated property 'wrapped' can not be referenced from a nonisolated context", 2 "call to actor-isolated instance method 'attachmentStreamURL(id:)' in a synchronous main actor-isolated context", 2 "main actor-isolated static property ... can not be referenced from a nonisolated context", 1 "call to main actor-isolated initializer 'init(ianaName:)'"). Files: Core/Calls/RingbackTone.swift (10), Core/Calls/WebRTCClient.swift (4), Core/LinkPreviewLoader.swift (2), Core/ShareImport.swift (1), Views/AttachmentViewer.swift (1), Views/AudioPlayerView.swift (1). Three of these are explicitly annotated "this is an error in the Swift 6 language mode"; the project is SWIFT_VERSION = 5.0 in every configuration, so they are warnings today and do not block this submission.
  - 2 deprecations: 'INStartAudioCallIntent' and 'INStartVideoCallIntent' deprecated in iOS 13.0, at Core/Calls/CallIntents.swift:46 and :48. These are deprecated, NOT removed, and the SDK still delivers them; they do not break the build or the upload.
  - 1 no-op cast: Core/MediaPrep.swift:468 "conditional downcast from 'Int?' to 'Int' does nothing".
  - 1 tool warning: appintentsmetadataprocessor "Metadata extraction skipped. No AppIntents.framework dependency found." Investigated — this is benign here: the app uses legacy SiriKit intents, Info.plist:44 declares INIntentsSupported = [INStartCallIntent], and all 10 AppIntentVocabulary.plist files (IntentName = INStartCallIntent) ARE present in the built bundle under Base/en/ru/de/es/fr/ja/sr/sr-Latn/zh-Hans .lproj. The ITMS-90626 fix looks intact in the product, not just in the source tree.
  - ZERO unreachable-code warnings, ZERO missing-switch-case warnings, ZERO Sendable-conformance warnings, ZERO deprecation warnings that reference a removed API.

BUILD NUMBER: project.pbxproj IS now dirty — CURRENT_PROJECT_VERSION 101 -> 103 across all 12 configuration blocks (git diff --stat: 12 insertions, 12 deletions). This was NOT caused by my three commands: the agvtool bump post-action lives only under <ArchiveAction> in both shared schemes (FamilyConnect.xcscheme:98-118, FamilyConnect-nettrash.xcscheme:107-127), there is no PBXShellScriptBuildPhase and no 'agvtool' string anywhere in project.pbxproj, and none of my three logs contain the word agvtool. The scratchpad also holds ios-archive.log and mac-archive.log written by concurrent sibling agents at 19:51/19:52 — two archive runs, two bumps. Expected behavior, not a defect. Nothing was committed or pushed; no source file was modified.

<details><summary>Commands run</summary>

- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=iOS Simulator,name=iPhone 16 Pro' -derivedDataPath <scratch>/dd-ios `  
  → FAILED (exit 70) - not a code failure: 'Unable to find a device matching the provided destination specifier: { platform:iOS Simulator, OS:latest, name:iPhone 16 Pro }'. iPhone 16 Pro exists here only on runtimes 18.1/18.2/18.4/18.6; OS:latest is 26.5. Log: <scratch>/ios-build.log (overwritten by the retry).
- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect-nettrash -destination 'platform=iOS Simulator,name=iPhone 16 Pro' -derivedDataPath <scratch`  
  → FAILED (exit 70) - same destination-resolution failure, no compilation attempted.
- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=iOS Simulator,id=9A611E4C-7F9D-43B1-9A5F-686CD77CB6F5' -derivedDataP`  
  → SUCCEEDED (exit 0). ** BUILD SUCCEEDED **. 0 errors, 26 'warning:' lines. Destination = iPhone 17, iOS 26.5. Full log: /private/tmp/claude-501/-Users-nettrash-Develop-nettrash-me/fb26281e-02e2-4a33-b709-b88acf252442/scratchpad/ios-build.log
- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect-nettrash -destination 'platform=iOS Simulator,id=9A611E4C-7F9D-43B1-9A5F-686CD77CB6F5' -der`  
  → SUCCEEDED (exit 0). ** BUILD SUCCEEDED **. 0 errors, 26 'warning:' lines (identical set to Debug). Confirmed built into Build/Products/Release-nettrash-iphonesimulator. Full log: /private/tmp/claude-501/-Users-nettrash-Develop-nettrash-me/fb26281e-02e2-4a33-b709-b88acf252442/scratchpad/ios-release.log
- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild test -project FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=iOS Simulator,id=9A611E4C-7F9D-43B1-9A5F-686CD77CB6F5' -only-testing:`  
  → SUCCEEDED (exit 0). ** TEST SUCCEEDED **. 615 tests / 617 test runs, 617 passed, 0 failed, 0 skipped, 0 expected failures. 19.9s. Full log: /private/tmp/claude-501/-Users-nettrash-Develop-nettrash-me/fb26281e-02e2-4a33-b709-b88acf252442/scratchpad/ios-test.log
- `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun xcresulttool get test-results summary --path <scratch>/dd-ios/Logs/Test/Test-FamilyConnect-2026.08.28_19-57-08-+0200.xcresult`  
  → SUCCEEDED - result "Passed"; passedTests 617 (per-device) / 615 (unique), failedTests 0, skippedTests 0, expectedFailures 0, testFailures []. Device iPhone 17, iOS 26.5 build 23F77, arm64.
- `xcodebuild test -only-testing:FamilyConnectUITests`  
  → SKIPPED - instructed not to run; UI tests require a live server and a signed build.
- `/usr/libexec/PlistBuddy -c Print <scratch>/dd-rel/Build/Products/Release-nettrash-iphonesimulator/FamilyConnect.app/Info.plist | grep -i 'FCDefault|CFBundleVersion|CFBundleShortVersion'`  
  → SUCCEEDED - FCDefaultServerURL = https://fc.nettrash.me, CFBundleShortVersionString = 1.0, CFBundleVersion = 103. Confirms Release-nettrash bakes the server URL.
- `ls <scratch>/dd-rel/Build/Products/Release-nettrash-iphonesimulator/FamilyConnect.app/{PlugIns,Frameworks} ; find ... -name AppIntentVocabulary.plist`  
  → SUCCEEDED - PlugIns/FamilyConnectShareExtension.appex and Frameworks/WebRTC.framework both embedded; all 10 AppIntentVocabulary.plist files present in the bundle (Base, en, ru, de, es, fr, ja, sr, sr-Latn, zh-Hans).
- `git status --porcelain ; git diff -U0 ios/FamilyConnect.xcodeproj/project.pbxproj`  
  → SUCCEEDED - only ios/FamilyConnect.xcodeproj/project.pbxproj modified: CURRENT_PROJECT_VERSION 101 -> 103 in all 12 configuration blocks (12 ins / 12 del). No source file touched, nothing committed or pushed.
- `grep -o '\.swift:[0-9]*:[0-9]*: warning: .*' ios-release.log | sort -u | ... uniq -c  ; diff <(...ios-build.log) <(...ios-release.log)`  
  → SUCCEEDED - 23 unique warning sites, 25 compiler emissions + 1 appintentsmetadataprocessor tool warning = 26 'warning:' lines. Debug and Release-nettrash warning sets are IDENTICAL (diff empty).
- `grep -n 'SWIFT_VERSION|SWIFT_APPROACHABLE_CONCURRENCY|SWIFT_DEFAULT_ACTOR_ISOLATION' project.pbxproj`  
  → SUCCEEDED - SWIFT_VERSION = 5.0 in all 12 configuration blocks; SWIFT_APPROACHABLE_CONCURRENCY = YES; SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor on the app/test targets. So the 'error in the Swift 6 language mode' warnings are warnings only.
- `grep -n 'PostActions|BuildAction|ArchiveAction|buildConfiguration' FamilyConnect*.xcscheme ; grep -n 'shellScript|agvtool' project.pbxproj`  
  → SUCCEEDED - agvtool bump post-action exists ONLY under <ArchiveAction> in both shared schemes; no PBXShellScriptBuildPhase and no 'agvtool' anywhere in project.pbxproj. FamilyConnect-nettrash: Build/Launch/Profile/Archive = Release-nettrash, Test/Analyze = Debug.

</details>

### `macos-build-test`
The macOS half of the shared target is in far better shape than expected: BOTH schemes build clean for platform=macOS (FamilyConnect/Debug and FamilyConnect-nettrash/Release-nettrash, ** BUILD SUCCEEDED **), the unit-test target does support macOS and runs 607 test cases with 0 failures on "My Mac", the app bundle is correctly laid out (Contents/PlugIns/FamilyConnectShareExtension.appex, Contents/Frameworks/WebRTC.framework with the macOS slice, AppIcon.icns, all 10 .lproj, PrivacyInfo.xcprivacy), the macOS Info.plist (Info-macOS.plist) is the one used and carries NO UIBackgroundModes, and the macOS rpath trap is already handled — LD_RUNPATH_SEARCH_PATHS[sdk=macosx*] = @executable_path/../Frameworks is set on the app target and otool confirms it in the binary. I also did a real signed build with the developer identity (BUILD SUCCEEDED, codesign --verify --deep --strict passes) and LAUNCHED it: it ran 15 s with no dyld failure and no crash report, so the standalone-Mac-launch failure mode is disproved. Two claims I expected to find were checked and disproved: the App Group id needs no team-ID prefix here (the Mac Store provisioning profile literally grants "group.me.nettrash.FamilyConnect", and ~/Library/Group Containers/group.me.nettrash.FamilyConnect exists), and arm64-only output was an artifact of -destination 'platform=macOS' (ARCHS = arm64 x86_64 for generic/platform=macOS). The one real defect is the push entitlement: FamilyConnect-macOS.entitlements uses the iOS spelling aps-environment, which macOS does not recognise, so the key is stripped at signing and the shipped Mac app has NO push entitlement at all — confirmed both in my signed build and in the already-exported App Store copy at /Applications/FamilyConnect.app (build 100). Beyond that the gap is documentation: README.md and ios/docs/appstore.md contain ZERO mentions of macOS, even though the Mac app is real, installed, and has Mac Team Store provisioning profiles. NOTE: the scheme Build PostAction ran agvtool as expected — CURRENT_PROJECT_VERSION went 101 -> 103 and ios/FamilyConnect.xcodeproj/project.pbxproj is now the only modified file in the tree. I made no other repo change and committed nothing.

<details><summary>Commands run</summary>

- `DEVELOPER_DIR=… xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=macOS' -derivedDataPath …/dd-mac CODE_SIGNING_ALLOWED=NO`  
  → SUCCEEDED (exit 0, ** BUILD SUCCEEDED **). Used FamilyConnect/Info-macOS.plist; embedded FamilyConnectShareExtension.appex into Contents/PlugIns and WebRTC.framework into Contents/Frameworks. Log: …/scratchpad/mac-build.log
- `DEVELOPER_DIR=… xcodebuild build -scheme FamilyConnect-nettrash -destination 'platform=macOS' -derivedDataPath …/dd-mac-nt CODE_SIGNING_ALLOWED=NO`  
  → SUCCEEDED (exit 0, ** BUILD SUCCEEDED **), configuration Release-nettrash, FCDefaultServerURL=https://fc.nettrash.me present in the built Info.plist. Log: …/scratchpad/mac-build-nt.log
- `DEVELOPER_DIR=… xcodebuild test -scheme FamilyConnect -destination 'platform=macOS' -only-testing:FamilyConnectTests -derivedDataPath …/dd-mac CODE_SIGNING_ALLOWED=NO`  
  → SUCCEEDED (exit 0, ** TEST SUCCEEDED **). 607 'Test case … passed' lines, 0 failed, 0 skipped, on 'My Mac - FamilyConnect'. The FamilyConnectTests target DOES support macOS (SUPPORTED_PLATFORMS = iphoneos iphonesimulator macosx). 42 compiler warnings (11 RingbackTone.swift, 4 WebRTCClient.swift, 4 CallIntents.swift deprecated INStartAudio/VideoCallIntent, rest concurrency). Log: …/scratchpad/mac-test.log
- `DEVELOPER_DIR=… xcodebuild test -scheme FamilyConnect -destination 'platform=macOS' (whole scheme, no -only-testing)`  
  → SUCCEEDED (exit 0, ** TEST SUCCEEDED **), 607 passed / 0 failed, with the note 'Cannot test target FamilyConnectUITests on My Mac: … does not support … com.apple.platform.macosx'. So the default test action does NOT break on macOS; it silently drops the UI tests. Log: …/scratchpad/mac-test-full.log
- `DEVELOPER_DIR=… xcodebuild test … -only-testing:FamilyConnectUITests -destination 'platform=macOS'`  
  → FAILED (exit 70) — 'Cannot test target “FamilyConnectUITests” on “My Mac”: FamilyConnectUITests does not support My Mac's platform: com.apple.platform.macosx'. Confirms the UI-test target is iOS-only. Log: …/scratchpad/mac-uitest.log
- `grep -n LD_RUNPATH_SEARCH_PATHS FamilyConnect.xcodeproj/project.pbxproj + otool -l on the built macOS binaries`  
  → SUCCEEDED. App target (all 3 configs, pbxproj lines 557/610/819) sets LD_RUNPATH_SEARCH_PATHS[sdk=macosx*] = ($(inherited), @executable_path/../Frameworks); otool on Contents/MacOS/FamilyConnect shows LC_RPATH /usr/lib/swift, @executable_path/Frameworks, @executable_path/../Frameworks, and otool -L shows @rpath/WebRTC.framework/WebRTC. The known rpath trap is NOT present.
- `lipo -info / otool -l LC_BUILD_VERSION on Contents/Frameworks/WebRTC.framework/WebRTC`  
  → SUCCEEDED. Universal x86_64+arm64, platform 1 (macOS), minos 13.0, sdk 26.5 — the correct macOS slice of the XCFramework, below the app's LSMinimumSystemVersion 14.0.
- `xcodebuild -showBuildSettings -scheme FamilyConnect-nettrash -destination 'generic/platform=macOS'`  
  → SUCCEEDED. ARCHS = arm64 x86_64, ONLY_ACTIVE_ARCH = NO, MACOSX_DEPLOYMENT_TARGET = 14.0, INFOPLIST_FILE = FamilyConnect/Info-macOS.plist, CODE_SIGN_ENTITLEMENTS = FamilyConnect/FamilyConnect-macOS.entitlements, ENABLE_HARDENED_RUNTIME = NO. The arm64-only binary I first measured was an artifact of the concrete -destination, not a project defect.
- `plutil -p …/Release-nettrash/FamilyConnect.app/Contents/Info.plist`  
  → SUCCEEDED. CFBundleSupportedPlatforms=[MacOSX], LSMinimumSystemVersion=14.0, DTSDKName=macosx26.5 — the macOS plist was used. NO UIBackgroundModes. FCDefaultServerURL, CFBundleURLTypes (familyconnect://), NSAppTransportSecurity/NSAllowsLocalNetworking and all 5 usage strings present. Leftover iOS keys: UILaunchScreen, UISupportedInterfaceOrientations~iphone/~ipad, UIApplicationSceneManifest, UIApplicationSupportsIndirectInputEvents.
- `codesign -d --entitlements - on the CODE_SIGNING_ALLOWED=NO app`  
  → SKIPPED effectively — output was only 'Executable=…' (unsigned, as expected). Re-ran the whole build WITH signing instead (next row) to get real entitlements.
- `DEVELOPER_DIR=… xcodebuild build -scheme FamilyConnect-nettrash -destination 'platform=macOS' -derivedDataPath …/dd-mac-signed   (signing ENABLED)`  
  → SUCCEEDED (exit 0). Signed with 'Apple Development: … (3PY7B8NM9K)', team V4WM2SJ8Q9; appex, WebRTC.framework and .app all signed; codesign --verify --deep --strict --verbose=2 → 'valid on disk' + 'satisfies its Designated Requirement'. Log: …/scratchpad/mac-build-signed.log
- `codesign -d --entitlements - --xml on the SIGNED app and appex`  
  → SUCCEEDED. App: app-sandbox, application-groups[group.me.nettrash.FamilyConnect], network.client, network.server, device.camera, device.audio-input, personal-information.location, files.user-selected.read-only, get-task-allow — but NO aps entitlement of any spelling. Appex: app-sandbox + the same app group (correctly sandboxed, which a Mac .appex requires).
- `open -n …/dd-mac-signed/…/FamilyConnect.app ; wait 15s ; pgrep -lx FamilyConnect ; ls ~/Library/Logs/DiagnosticReports`  
  → SUCCEEDED — launched and still alive after 15 s (pid 19474 from the scratchpad path), no crash report, no dyld image-not-found. Killed it afterwards. Also revealed a pre-existing /Applications/FamilyConnect.app running as pid 1175, i.e. the Mac app is already installed and in use.
- `codesign -d --entitlements - /Applications/FamilyConnect.app ; plutil -p its Info.plist`  
  → SUCCEEDED. The already-exported Mac build (CFBundleVersion 100) carries beta-reports-active (an App Store/TestFlight export) and the same entitlement set — again with NO aps entitlement. This is the strongest evidence for finding macos-aps-entitlement-key.
- `security cms -D -i <Mac Team Store/Team Provisioning Profile>.provisionprofile | plutil -p`  
  → SUCCEEDED. Both Mac profiles for me.nettrash.FamilyConnect grant com.apple.developer.aps-environment (development / production) and com.apple.security.application-groups = ['group.me.nettrash.FamilyConnect'] — proving (a) the bare, non-team-prefixed app group is fine on macOS here, (b) the push capability exists on the App ID and only the entitlements KEY NAME is wrong.
- `ls ~/Library/Group\ Containers | grep -i family ; ls ~/Library/Containers | grep -i nettrash`  
  → SUCCEEDED. group.me.nettrash.FamilyConnect and me.nettrash.FamilyConnect.ShareExtension both exist — the Mac share-extension handoff container is real on this machine.
- `ls Contents/Resources of the built .app ; ls the appex`  
  → SUCCEEDED. AppIcon.icns, Assets.car, PrivacyInfo.xcprivacy, and all 10 .lproj (Base, en, ru, de, es, fr, ja, sr, sr-Latn, zh-Hans) each with AppIntentVocabulary.plist. The appex has no Resources dir (it is not localized).
- `grep -i 'macos|mac app|notariz|developer id' README.md ios/docs/appstore.md`  
  → SUCCEEDED with ZERO matches in either file — basis for finding macos-absent-from-release-docs.
- `grep CURRENT_PROJECT_VERSION project.pbxproj ; git status --porcelain`  
  → SUCCEEDED. Now 103 (was 101) and 'M ios/FamilyConnect.xcodeproj/project.pbxproj' — the expected agvtool bump from the scheme Build PostAction, nothing else changed, nothing committed.

</details>

### `archive-validate`
Both archives CAN be produced today, and both pass Xcode's own `-validate-for-store` step: `** ARCHIVE SUCCEEDED **` for iOS (generic/platform=iOS) and macOS (generic/platform=macOS), each signed end-to-end with a real identity and each verifying clean under `codesign --verify --deep --strict` ("valid on disk / satisfies its Designated Requirement"). I found NO hard blocker to assembling or uploading a binary. Everything that usually breaks at archive time is in order: iOS app arm64-only with a 1024x1024 AppIcon rendition compiled into Assets.car (plus AppIcon60x60@2x / AppIcon76x76@2x~ipad extracted to the bundle); macOS ships a complete 16-1024 icon ladder plus a generated AppIcon.icns and correctly transparent (non-full-bleed) macOS icon art; the Share Extension is embedded on both platforms, its NSExtensionActivationRule is a sane dictionary, its principal class `_TtC27FamilyConnectShareExtension19ShareViewController` is really in the binary, and its entitlements carry `com.apple.security.app-sandbox` on both (so no error-90296-class non-sandboxed-appex rejection); WebRTC.framework is embedded with only the right slices (iOS arm64 only — no simulator or Mac Catalyst leak; macOS x86_64+arm64), carries a PrivacyInfo.xcprivacy, and is re-signed by Xcode; the macOS binary carries the `@executable_path/../Frameworks` rpath so it can actually find WebRTC at launch; dSYMs for app and appex are present in both archives; and MinimumOSVersion 17.0 / LSMinimumSystemVersion 14.0, LSApplicationCategoryType, UIRequiredDeviceCapabilities=[arm64], UILaunchScreen, CFBundleDisplayName "Family" and matching 1.0/101 version keys are all present. What I did find is four things that bite AFTER assembly: no `ITSAppUsesNonExemptEncryption` in either Info.plist (an export-compliance question on every single upload), the macOS push entitlement silently stripped from the signed binary because the entitlements file uses the iOS spelling, the macOS Save-attachment path writing through an NSSavePanel with only `user-selected.read-only`, and an alpha channel in the iOS App Store icon source PNG. Two process facts: this machine has ONLY an "Apple Development" identity, so `-exportArchive` for App Store and any altool validation cannot be run here; and the two archives bumped CURRENT_PROJECT_VERSION 101 -> 103 (expected, per the scheme's `agvtool bump` PostAction) leaving `ios/FamilyConnect.xcodeproj/project.pbxproj` as the only modified file.

<details><summary>Commands run</summary>

- `DEVELOPER_DIR=... xcodebuild archive -scheme FamilyConnect-nettrash -destination 'generic/platform=iOS' -archivePath .../FC-ios.xcarchive`  
  → SUCCEEDED — ** ARCHIVE SUCCEEDED **, signed "Apple Development: Ivan Alekseev (3PY7B8NM9K)" with "iOS Team Provisioning Profile: me.nettrash.FamilyConnect"; builtin-validationUtility -validate-for-store and embeddedBinaryValidationUtility both ran without error; 26 compiler warnings, 0 errors
- `DEVELOPER_DIR=... xcodebuild archive -scheme FamilyConnect-nettrash -destination 'generic/platform=macOS' -archivePath .../FC-mac.xcarchive`  
  → SUCCEEDED — ** ARCHIVE SUCCEEDED **, same identity with "Mac Team Provisioning Profile: me.nettrash.FamilyConnect"; builtin-validationUtility -validate-for-store ran without error; 42 compiler warnings, 0 errors
- `plutil -p FC-ios.xcarchive/Products/Applications/FamilyConnect.app/Info.plist  (and the macOS Contents/Info.plist)`  
  → SUCCEEDED — all store-relevant keys present EXCEPT ITSAppUsesNonExemptEncryption (absent in both)
- `xcrun assetutil --info <Assets.car> for both archives`  
  → SUCCEEDED — iOS: AppIcon 1024x1024 renditions for phone+pad idioms, Encoding ARGB, Opaque=True. macOS: full 16/32/32/64/128/256/256/512/512/1024 ladder, all with alpha as macOS requires
- `sips -g pixelWidth -g pixelHeight on all 11 PNGs in Assets.xcassets/AppIcon.appiconset`  
  → SUCCEEDED — every declared size matches its actual pixels (AppIcon.png 1024x1024; AppIcon-macOS-512x512@2x.png 1024x1024; etc.)
- `python3 PNG IHDR + IDAT decode of AppIcon.png (colour type, per-pixel min alpha)`  
  → SUCCEEDED — colour type 6 (RGBA, alpha channel present), min alpha across all 1,048,576 pixels = 255 (fully opaque)
- `codesign -d --entitlements :- on both apps and both .appex bundles`  
  → SUCCEEDED — iOS app: aps-environment=development, com.apple.developer.siri, app group, get-task-allow. macOS app: sandbox + network.client/server + camera + audio-input + location + user-selected.read-only + app group, and NO push entitlement. Both appex: app-sandbox + app group
- `plutil -p <ArchiveIntermediates>/.../FamilyConnect.app.xcent (macOS)`  
  → SUCCEEDED — confirms aps-environment was dropped during entitlement generation, not merely absent from the dump
- `security cms -D -i embedded.mobileprovision / embedded.provisionprofile`  
  → SUCCEEDED — iOS: development profile, 5 ProvisionedDevices, expires 2027-08-28. macOS: Mac Team Provisioning Profile granting com.apple.developer.aps-environment=development
- `codesign --verify --deep --strict --verbose=2 on both .app bundles`  
  → SUCCEEDED — both "valid on disk" and "satisfies its Designated Requirement"
- `lipo -info on both app binaries and both embedded WebRTC binaries`  
  → SUCCEEDED — iOS app arm64; iOS WebRTC arm64 only; macOS app and WebRTC both x86_64+arm64. No simulator or Catalyst slice in either archive
- `codesign -dvvv on SourcePackages/artifacts/webrtc/WebRTC/WebRTC.xcframework/ios-arm64/WebRTC.framework`  
  → SUCCEEDED (as a query) — reports "code object is not signed at all": the upstream stasel/WebRTC 151.0.0 binary ships unsigned; Xcode re-signs it at embed
- `otool -l | grep LC_RPATH on the macOS app binary`  
  → SUCCEEDED — @executable_path/../Frameworks present alongside @executable_path/Frameworks, so the standalone Mac launch will resolve @rpath/WebRTC.framework/WebRTC
- `plutil -p / plutil -lint on all 10 AppIntentVocabulary.plist files in the iOS archive`  
  → SUCCEEDED — all 10 lint OK, each declares IntentName=INStartCallIntent matching the app Info.plist INIntentsSupported entry
- `strings/otool on FamilyConnectShareExtension binary for the NSExtensionPrincipalClass`  
  → SUCCEEDED — _TtC27FamilyConnectShareExtension19ShareViewController is present, so the declared principal class resolves
- `xcodebuild -exportArchive -exportOptionsPlist (method app-store-connect) on FC-ios.xcarchive`  
  → FAILED (expected, environment) — 'error: exportArchive No signing certificate "iOS Distribution" found' and 'DVTDeveloperAccountManager: ... missing Xcode-Username'. No .ipa produced
- `security find-identity -v -p codesigning`  
  → SUCCEEDED — exactly 1 identity: "Apple Development: Ivan Alekseev (3PY7B8NM9K)". No Apple Distribution / 3rd Party Mac Developer certificate
- `xcrun --find altool ; xcrun altool --validate-app`  
  → SKIPPED as a real validation — altool and notarytool exist at /Applications/Xcode.app/Contents/Developer/usr/bin/, but --validate-app needs a built .ipa plus App Store Connect credentials; the probe returned 'ERROR: [altool] Cannot expand files with extension' (-19239). Nothing was uploaded or notarized
- `grep -rn required-reason API symbols across FamilyConnect + FamilyConnectShareExtension`  
  → SUCCEEDED — 0 hits for contentModificationDate/creationDate/systemUptime/mach_absolute_time/disk-space APIs; 1 attributesOfItem (MediaPrep.swift:468, reads .size only); UserDefaults used only in the app, which PrivacyInfo.xcprivacy already declares with reason CA92.1. Privacy manifest looks adequate; the appex uses no required-reason API
- `git -C .../family.connect status --short`  
  → SUCCEEDED — only ' M ios/FamilyConnect.xcodeproj/project.pbxproj' (CURRENT_PROJECT_VERSION 101 -> 103 from the two archives' agvtool bump PostAction). No other file touched by me

</details>

---
## Blockers (5 — 5 resolved in code; provisioning and the console form remain)
These gate submission. Ranked hardest first.

### 1. ~~No way to report content or block a member~~ — RESOLVED 2026-08-30 — [#1](https://github.com/nettrash/family.connect/issues/1)
**Platform:** both · **Effort:** multiple days · **Status: code complete on iOS, macOS, Android and the server.**
See "Status update — 2026-08-30" above. The finding below is the 2026-08-28 record and is left
unedited; closing the issue still needs #4's review notes and #2's published contact.

**What.** There is no Report, no Block, no mute and no content filtering anywhere in the client or the protocol. Repo-wide greps over ios/FamilyConnect (including all 447 keys of Localizable.xcstrings) and server/src/app.rs's full 44-route table return nothing; the only moderation control is owner-only member removal (ios/FamilyConnect/Views/FamilyManageView.swift:281-291, gated on `session.isOwner, member.role != "owner"`). A non-owner being harassed can only leave the family (SettingsView.swift:297) or delete their account, and the family owner cannot be removed by anyone. Registration on the shipped default server is completely ungated (server/src/handlers_auth.rs:114-155 — no invite, no email, no allowlist, no config switch), and your own review script hands the reviewer an instant-join invite code into a seeded family that already contains other people's message history.

**Why it blocks.** Guideline 1.2 requires apps with user-generated content to provide content filtering, a report mechanism with timely responses, the ability to block abusive users, and published contact information. Two of those four — report and block — cannot be satisfied by metadata or review notes; they need code. A reviewer will long-press a message (Reply / Edit / Copy / Share), open a member row (Link to a Contact… / Unlink Contact), find neither, and file the standard 1.2 rejection. There is a genuine mitigating argument — a stranger who registers can reach nobody (server/src/handlers_chat.rs:1311-1316 refuses cross-family direct chats, there is no user directory, joining needs an 8-char code from a 30-symbol alphabet and the DB default join_policy is 'approval') — but it does not cover intra-family abuse, and the reviewer is deliberately placed inside a shared UGC chat.

**Fix.** Minimum that clears both hard legs: (1) a per-member Block reachable from the member roster and from a message — client-side hide plus a server-side mute so a blocked member's messages and calls do not arrive; (2) a Report action on a message and on a member posting to a new POST /api/v1/reports, with a stated response commitment in the App Review notes. Then rewrite the review notes to lead with the containment argument (invite-code-only membership, approval by default, no cross-family contact, owner can remove, member can leave) and to stop advertising "Registration is also fully open" (appstore.md:52) as a feature. If you want to shrink the surface further, add the `[registration]` server switch your own checklist already proposes (appstore.md:102) and close the box to strangers.

### 2. ~~No privacy policy page, no support page, and no in-app privacy link~~ — RESOLVED 2026-08-30 — [#2](https://github.com/nettrash/family.connect/issues/2)
**Platform:** both · **Effort:** a day

**Resolved.** Four pages now exist in the `nettrash-me` repo — `frontend/assets/appstore/familyconnect/{privacy,support}.html` for the Apple builds and `frontend/assets/play/familyconnect/{privacy,support}.html` for the Play build. They are not copies of each other: the Android pair names FCM rather than APNs, Google rather than Apple for map tiles, the `READ_MEDIA_*` storage permission rather than a Photos picker, and drops the Siri/iCloud paragraph, because a policy that describes the wrong platform's data flows is worse than none. Both pairs cover the two operating modes (the developer-run `fc.nettrash.me` the store build points at, vs. a self-hosted server), plaintext PostgreSQL storage, the retention window, the deletion semantics, STUN peer-IP exposure, and Azure OpenAI when `[ai]` is on. Store cards were added to `home.rs` and the two loose icon PNGs given explicit `copy-file` lines — `copy-dir` covered only the HTML directories, so the icons were reaching the site as 404s.

In-app links are in place on all three clients: `SettingsView.swift:368` and `:371`, `MacSettingsView.swift:109` and `:112`, and the Android `SettingsScreen.kt` Privacy section, which had none until this pass and points at the `/play/` pair.

**Left for you:** deploy nettrash.me, then paste the URLs into App Store Connect (App Information → Privacy Policy, and version metadata → Support URL) and into the Play Console listing. Also set `support_contact` on the live server — #1's report sheet shows it as the escalation path.

**What.** Nothing for Family Connect exists on nettrash.me: frontend/assets/appstore/ contains only exchange, geo, md, scan, and a repo-wide grep for familyconnect / "family connect" returns zero files. No policy text is drafted anywhere in the family.connect repo either (`find` for *privacy*/*support* returns only ios/FamilyConnect/PrivacyInfo.xcprivacy, which is the API manifest, not a policy). Inside the app there is no privacy-policy link and no support contact on either platform — SettingsView.swift:312-327 and MacSettingsView.swift:95-110 have a section headed "Privacy" that is only the Link/Map preview toggles, and Localizable.xcstrings has no "Privacy Policy" key in any of the 9 languages. ios/docs/appstore.md:70 and :100 still carry the literal [SUPPORT_EMAIL] placeholder.

**Why it blocks.** Privacy Policy URL is a required App Information field and Support URL is required version metadata; App Store Connect will not let you Submit for Review with either empty, and this applies independently to the iOS and macOS platform records. Guideline 5.1.1(i) additionally requires the privacy-policy link to be reachable inside the app, which it is not. You also cannot point at a sibling app's page: nettrash.me's nginx SPA fallback returns HTTP 200 with the site shell for any /appstore/<anything>/ path, so a plausible-looking URL would silently land the reviewer on your homepage rather than 404ing.

**Fix.** Write frontend/assets/appstore/familyconnect/{privacy,support}.html in the nettrash-me repo, modelled on scan/privacy.html, and add the card to frontend/src/components/home.rs. The policy must cover both modes explicitly (the developer-operated default server https://fc.nettrash.me that the store build ships pointed at, vs. a self-hosted server where you receive nothing) and name every category actually collected: message text, photos, videos, voice notes, arbitrary files, precise WGS-84 coordinates, avatars, username, display name, birthday day/month, APNs/PushKit tokens; plus plaintext storage in PostgreSQL (README.md:159 — do not imply E2E), server-side retention (retention_days default 100, which deletes messages AND their media), account-deletion semantics (POST /api/v1/me/delete scrubs the user but keeps family-chat messages as "Deleted account"), Google's public STUN default seeing peer IPs during calls, and Azure OpenAI if [ai] is enabled on your box. Then add a Privacy Policy row to SettingsView and MacSettingsView linking the published URL, and fill both URLs plus the support email in App Store Connect.

### 3. ~~No screenshots exist for any Apple device class~~ — RESOLVED for iOS/iPad/Play 2026-08-30, macOS still open — [#3](https://github.com/nettrash/family.connect/issues/3)
**Platform:** both · **Effort:** a day

**Resolved for the three sets that gate the iOS and Play submissions.** iPad was kept (`TARGETED_DEVICE_FAMILY` stays `"1,2"`), so all three sets exist:

| Set | Where | Size | How to regenerate |
| --- | --- | --- | --- |
| iPhone 6.9" ×6 | `ios/docs/screenshots/iphone-6.9/` | 1320×2868 | `xcodebuild test -only-testing:FamilyConnectUITests/StoreScreenshotUITests`, then `ios/scripts/export-screenshots.sh` |
| iPad 13" ×6 | `ios/docs/screenshots/ipad-13/` | 2064×2752 | same, `-destination` iPad Pro 13-inch (M4) |
| Play phone ×6 | `android/fastlane/.../phoneScreenshots/` | 1080×1920 | `android/scripts/capture-play-screenshots.sh start` then `screens` |
| Mac ×4 | `ios/docs/screenshots/mac/` | 2560×1600 | `ios/scripts/capture-mac-screenshot.sh start`, then `shot <name> [window]` |

All of it runs against one seeded family — `server/scripts/seed-store-screenshots.sh` builds "The Harpers": five members, a thirteen-message thread with reactions and replies, a four-photo album, a poll mid-vote, a shared location, four board notes and a direct chat, all backdated so the timestamps read like a week of family life rather than one minute of scripting.

**The three Android screenshots that were committed in August are deleted, not updated.** They were 1344×2992 — a 2.226:1 ratio, past Play's 2:1 ceiling — and RGBA, where the spec asks for 24-bit with no alpha. Both faults came from the device they were taken on, so the capture script uses its own AVD pinned to 1080×1920 rather than resizing somebody's Pixel_8_Pro, and every shot is flattened and then checked against both rules before it is written. `featureGraphic.png` carried the same alpha channel and is now RGB (its alpha was fully opaque, so nothing was lost).

**The Mac set now exists too** — four shots at 2560×1600 (family chat, board, family, settings),
captured from a build with a throwaway bundle id so signing it into the demo server could not touch
the real app's preferences, Keychain or cache. Two things are worth knowing before that set is
used. A window-only capture holds the window's own pixels and nothing behind it, so the translucent
header and footer bars of the Family and Settings sheets render flat grey; the board's grey canvas
is NOT that artefact — it is `Color(nsColor: .underPageBackgroundColor)` and genuinely grey. And
the direct-chat screen could not be reached: the SwiftUI sidebar exposes no rows to accessibility
and ignores both synthetic clicks and arrow keys, so there is no fifth shot rather than a
mislabelled duplicate of the first.

**Left for you:** retake the two sheet screens by hand if the Mac listing ships, since only a
screen-region capture renders those materials correctly. Optionally drop real photographs into `server/scripts/screenshot-photos/` and re-run — with that directory empty the seeder generates gradients, which is what the album tiles in the current shots are.

**What.** `find` for any *screenshot* path or any .png/.jpg outside Assets.xcassets returns exactly one hit in the whole repo: android/fastlane/metadata/android/en-US/images/phoneScreenshots (1344x2992, Compose UI, unusable for Apple). There is no fastlane/deliver setup under ios/. Meanwhile the shipped iOS archive's Info.plist carries UIDeviceFamily = [1, 2] and UISupportedInterfaceOrientations~ipad with all four orientations (project.pbxproj TARGETED_DEVICE_FAMILY = "1,2" in all 12 configurations), so the iPad slot is required in addition to iPhone. ios/docs/appstore.md's pre-submission checklist never mentions screenshots at all.

**Why it blocks.** App Store Connect will not enable Submit for Review until at least one screenshot exists for every required display size. Capturing the iPad set will also force you to look at the iPad layout, which is an unadapted full-width iPhone UI: the only NavigationSplitView in the codebase is MacViews/MacChatView.swift:51, and a repo-wide grep for horizontalSizeClass / userInterfaceIdiom returns zero hits, so a 13-inch iPad renders the single-column phone chat list edge to edge with uncapped text balloons.

**Fix.** Seed the reviewer family with content worth photographing (a family thread, a photo album, a voice message, a poll, a call record), then capture the 6.9" iPhone set on iPhone 16 Pro Max or 16 Plus (NOT iPhone 16 Pro — that is the 6.3" class and is rejected for the 6.9" slot) and the 13" iPad set on iPad Pro 13-inch (M4). Screenshots are required only for the primary localization, not all 9. If you decide iPad is not ready, set TARGETED_DEVICE_FAMILY = "1" instead and the iPad slot disappears.

### 4. ~~ios/docs/appstore.md is a stale draft~~ — REWRITTEN 2026-08-30, provisioning still outstanding — [#4](https://github.com/nettrash/family.connect/issues/4)
**Platform:** both · **Effort:** hours

**Rewritten in full against the shipped feature set**, with every claim traced to code rather than
carried forward. The file now states and enforces the character limits App Store Connect applies at
entry — Promotional Text 168/170, Description 3985/4000, Keywords 95/100, App Review notes
3730/4000 — all measured against the installed file, not estimated. The old Notes for App Review
would have been 18,224 characters against a 4,000-character field, so the section is split into a
paste-verbatim block and a reviewer walkthrough that stays in the repo.

Four things changed the submission, not just the prose:

1. **Guideline 1.2 framing.** Both the draft description and the draft notes opened by volunteering
   that no content is scanned automatically. 1.2 asks UGC apps for a filtering method, so that is a
   written admission against the requirement. The file now leads with containment — no feed, no
   discovery, no directory, contact bounded by a membership the owner controls — and presents
   Report, Block, the owner's inbox and member removal as the response.
2. **The child-audience signal is gone.** "kids" as a keyword and "grandparents to kids" opening the
   description presented an unfiltered messenger as child-directed, which invites a 1.3 and
   age-rating problem.
3. **Calls are not family-gated.** A direct chat outlives family membership, so the gate is
   direct-chat membership. The claim that every path to a person is family-gated was false.
4. **No unconditional relay claim.** `turn_urls` defaults empty and `config.rs` says calls that
   cannot connect directly simply fail, so every relay sentence is now conditional on the operator
   having wired one.

Three defects surfaced while fact-checking, all now checklist items in that file:

- **ITMS-91053 at upload.** `MediaPrep.swift:468` calls `attributesOfItem(atPath:)`, a
  required-reason API, and `PrivacyInfo.xcprivacy` declares no `NSPrivacyAccessedAPICategoryFileTimestamp`
  entry. This fails before a human sees the build.
- **A reported caption-less photo freezes an empty excerpt** in the owner's report inbox, because the
  server freezes `messages.body`, which is empty for a media message with no caption — and
  `protocol.md` promises a `message_attachments` field the query never selects. That is exactly the
  path a reviewer probing 1.2 walks.
- **`PrivacyInfo.xcprivacy` declares an empty `NSPrivacyCollectedDataTypes`** — the binary asserts it
  collects nothing while the notes say the developer holds the data. Apple compares the two.

**Left for you:** provision the two demo accounts and the seeded reviewer family on fc.nettrash.me
and substitute the five placeholders; read the live `config.toml` and write six values into the
notes rather than assuming defaults (`[ai]`, `[calls] enabled`/`video_enabled`, `retention_days`,
`stun_urls`, `turn_urls`/`turn_secret`, `include_message_body`); and settle export compliance, the
age rating, the iPad question and the store name.

**What.** The file was last touched 2026-08-20, 76 commits ago, before media, calls, the share extension and macOS landed. It states "the honest limits of version 1: text messages only. Voice and video calls are planned" (:31), "Text messages only in this version." (:66), "photos and other media, and voice/video calls... not here yet" (:80), and "no third-party SDKs of any kind" (:31, :63) — the last flatly false, since project.pbxproj:1050 embeds stasel/WebRTC 151.0.0 and WebRTC.framework is in both archives' Frameworks dir. It opens with a "BLOCKER before submitting (guideline 5.1.1(v))" banner saying account deletion does not exist, and repeats it as unchecked checklist item :96 — both stale, since server/src/app.rs:41 routes POST /api/v1/me/delete and DeleteAccountView.swift ships on both platforms. Six placeholders remain unfilled: [DEMO_USER]/[DEMO_PASS] (:49), [DEMO_USER_2]/[DEMO_PASS_2] (:50), [INVITE_CODE] (:57), [SUPPORT_EMAIL] (:70).

**Why it blocks.** This is the only source for the Description, the App Review notes and the TestFlight copy. Pasting it as-is puts "username [DEMO_USER]" into App Review Information, and hands the reviewer written statements the binary contradicts within seconds — the build declares NSCamera, NSMicrophone, NSLocationWhenInUse, NSPhotoLibraryAdd and NSLocalNetwork usage strings whose own text talks about video calls, voice messages and location sharing, and Info.plist declares UIBackgroundModes audio+voip. That mismatch is the ordinary cause of a Guideline 2.1 "we need more information" round. Separately, the demo accounts and reviewer family have to actually exist on fc.nettrash.me — the server is up (GET /api/v1/healthz returns 200) but there is no way to confirm the accounts from outside.

**Fix.** Rewrite Description, Keywords, Promotional Text, Beta App Description, What to Test and the Notes for App Review against the shipped feature set: family + 1:1 chats, photos/videos/albums with a viewer, voice messages, file attachments, the Share Extension, location sharing, polls, boards, reactions, replies, edits, read receipts, typing indicators, offline history, push, and P2P voice AND video calls with CallKit. Replace "no third-party SDKs of any kind" with the accurate line (no ads/analytics/tracking; one third-party library, Google's WebRTC, used only as the call media engine). Delete the account-deletion blockquote (:5-8) and checklist item (:96), and fix :60, which wrongly says deletion removes the account "and its messages" — the server scrubs the user and keeps family-chat messages as "Deleted account". Extend HOW TO REVIEW to walk media, a voice message, a poll, location, the Share Extension and a two-device call, and to explain the camera/mic/location/local-network prompts. Then provision the two demo accounts plus a seeded reviewer family on fc.nettrash.me and substitute every placeholder, including the owner credentials in App Review Information.

### 5. ~~App Privacy answers are a mandatory submission gate and the answer written down in your own doc is wrong~~ — CODE DONE 2026-08-30, form is yours to file — [#5](https://github.com/nettrash/family.connect/issues/5)

**`PrivacyInfo.xcprivacy` now tells the truth**, and is verified present in a built bundle with ten
data types — message bodies, photos/videos, audio, other user content, precise location, user id,
device id, name, other data (the birthday) and product interaction — all Linked, none Tracking, all
App Functionality. `ios/docs/appstore.md` lists the same constants so the manifest and the console
answers cannot drift apart.

**Two defects were fixed that the issue did not know about**, both found by checking Apple's
documentation rather than trusting the audit's own summary:

- The manifest declared **no `NSPrivacyAccessedAPICategoryFileTimestamp` entry at all**, while
  `MediaPrep.fileSize(of:)` calls `FileManager.attributesOfItem(atPath:)` on every attachment and
  recording path. An undeclared required-reason API is an **ITMS-91053 rejection at upload** — the
  build would never have reached a human reviewer. Now declared with `DDA9.1` (app container) and
  `C617.1` (a file the person picked).
- The User defaults reason was **`CA92.1`, the App Group code**, for defaults the app does not
  share — it uses `UserDefaults.standard` and shares no suite with the Share Extension. Corrected
  to `54BD.1`.

Worth noting how nearly this went wrong: the sub-agents that researched it reported `C617.1` as the
app-container reason and `DDA9.1` as something else, which is backwards, and recommended `3B52.1` —
which is the reason for a *third-party SDK* acting on the app's behalf. Fetching Apple's own
documentation JSON settled it. Do not take reason codes from memory.

**Left for you:** file the form in App Store Connect against the list in `appstore.md`, and settle
the two inputs the repo cannot answer — whether `[ai]` is enabled on fc.nettrash.me (it changes the
disclosure, the policy text and the age rating), and export compliance now that WebRTC ships
BoringSSL, which puts the app outside the "only Apple's encryption" exemption.
**Platform:** both · **Effort:** hours

**What.** The App Privacy questionnaire must be completed before a version can be submitted. ios/docs/appstore.md:101 prescribes declaring only "User Content (messages) and User ID (username)" and asserts "Data Not Linked to You is reasonable since no email/phone/real name is required". Both halves are wrong for the shipping build: Release-nettrash bakes in FC_DEFAULT_SERVER_URL = https://fc.nettrash.me (project.pbxproj:769), a server you operate, and the app uploads photos/videos, voice notes, arbitrary files, precise coordinates (docs/protocol.md:788 sends latitude/longitude/accuracy_m), avatars, birthdays and push tokens to it — all keyed to a persistent server account, which is what "linked" means regardless of whether an email was required. The shipped ios/FamilyConnect/PrivacyInfo.xcprivacy tells the same wrong story with `NSPrivacyCollectedDataTypes` as an empty array.

**Why it blocks.** You cannot skip the form, and an inaccurate label on a privacy-positioned messenger is a post-review correction at best. It also depends on decisions you have not made yet (retention value on the live box, whether the Azure OpenAI assistant is enabled), which is why it has to come after the privacy policy text.

**Fix.** Declare, all App Functionality, none used for tracking, Linked = true: User Content (messages/polls/board notes), Photos or Videos, Audio Data, Other Data Types (arbitrary files), Precise Location, Identifiers (user id/username; push token as Other Data rather than advertising Device ID), and Contact Info only if you judge display_name to be a name. Do NOT declare Contacts — ContactPicker uses CNContactPickerViewController with no permission and ContactLinks stay in on-device UserDefaults, never reaching the server. Mirror the same set into PrivacyInfo.xcprivacy so Xcode's generated privacy report agrees with the filed label, and add a one-line note to the review notes explaining that the label describes the pre-configured default server and that a self-hosted user shares nothing with you.

---
## Should fix (18)
Real defects and risks. None gates submission; several would produce bad first-week reviews.

### Photos permanently stop loading after a single transient network error — [#6](https://github.com/nettrash/family.connect/issues/6)
**Platform:** both

**What.** ios/FamilyConnect/Core/AttachmentStore.swift:90 flattens a 404 and a timeout/refused-connection/5xx into the same nil via `try?`, and :92-101 inserts the key into `missing` for all of them. The gate at :68 (`guard !missing.contains(key)`) sits before both the disk-cache check and the fetch, and nothing clears `missing` except logout or message deletion. The result is a permanent spinner (AttachmentView.swift:110-113 keeps `isAwaitingBytes` true forever) for every photo whose fetch was attempted during an outage, for the rest of the process lifetime. The sibling AvatarStore already solves exactly this with an explicit `settled:` flag (AvatarStore.swift:96-124), whose comment names the failure mode.

**Fix.** Give AttachmentStore the same `settled` flag: catch a real 404 (nil return) as settled → insert into `missing`; treat every other error as unsettled → do not insert, and bump `generation` so the view retries.

### macOS ships with no push entitlement: the entitlements file uses the iOS key spelling — [#7](https://github.com/nettrash/family.connect/issues/7)
**Platform:** macos

**What.** ios/FamilyConnect/FamilyConnect-macOS.entitlements:25 declares `aps-environment`; macOS uses `com.apple.developer.aps-environment`, which is exactly what both Mac provisioning profiles grant. Xcode filters the request against the profile and silently drops it — the generated .xcent and `codesign -d --entitlements` on both the local archive and the already-distributed /Applications/FamilyConnect.app (build 100, beta-reports-active) show no push key of any spelling. PushRegistrar.swift:193 still calls NSApplication.shared.registerForRemoteNotifications(), which fails into a .info-level log. Net effect: a Mac that is quit receives no notifications at all; in-app banners still work because ChatNotifier raises them locally off the live socket.

**Fix.** Rename the key to `com.apple.developer.aps-environment` (keep value "development"), rebuild, and confirm with `codesign -d --entitlements - --xml <app>` that it survives packaging. Leave ios/FamilyConnect/FamilyConnect.entitlements alone.

### macOS Save… is a dead button — the sandbox grants only user-selected.read-only — [#8](https://github.com/nettrash/family.connect/issues/8)
**Platform:** macos

**What.** ios/FamilyConnect/FamilyConnect-macOS.entitlements:37 grants `com.apple.security.files.user-selected.read-only` and there is no read-write variant anywhere in the repo, while MacViews/MacAttachmentViewer.swift:160-167 opens an NSSavePanel and writes to the chosen URL with two `try?` calls that discard every error. Depending on the OS's Powerbox behaviour the panel either never presents or the copy is denied; either way the toolbar's "Save a copy" does nothing and says nothing. Share… (NSSharingServicePicker, :179) still works, so the file can be got out — it is one dead control, not a total loss.

**Fix.** Add `com.apple.security.files.user-selected.read-write` (it subsumes read, so MacFilePicker's NSOpenPanel keeps working) and replace the two `try?` with a do/catch that surfaces a failure the way MacConversationView's mediaNotice does.

### ITSAppUsesNonExemptEncryption is absent from both Info.plists — [#9](https://github.com/nettrash/family.connect/issues/9)
**Platform:** both

**What.** Zero hits repo-wide, in either plist and in no INFOPLIST_KEY_*; confirmed absent from both archived bundles. Every uploaded build therefore lands in App Store Connect flagged "Missing Compliance" and cannot be attached to a submission or sent to external TestFlight testers until the questionnaire is answered by hand — once per platform, per build. Three sibling apps in this workspace (Geo, md, md.macOS) set it explicitly with the reasoning written into their store docs; family.connect is the outlier.

**Fix.** Add `INFOPLIST_KEY_ITSAppUsesNonExemptEncryption = NO` so it lands in both platforms' plists. The app implements no crypto of its own (zero CryptoKit/CommonCrypto hits; KeychainStore is OS keychain only), so the exemption applies — but record in appstore.md that the encryption present is TLS via URLSession plus DTLS-SRTP inside the WebRTC XCFramework, which bundles its own BoringSSL rather than using Apple's, since that is the one fact that makes the answer non-trivial.

### attributesOfItemAtPath: is in the binary with no required-reason declaration — [#10](https://github.com/nettrash/family.connect/issues/10)
**Platform:** both

**What.** ios/FamilyConnect/Core/MediaPrep.swift:467-468 calls `FileManager.default.attributesOfItem(atPath:)[.size]`, and `attributesOfItemAtPath:error:` appears in the ObjC selector section of both the iOS device archive binary and the macOS archive binary. PrivacyInfo.xcprivacy declares only NSPrivacyAccessedAPICategoryUserDefaults / CA92.1. WebRTC.framework declares FileTimestamp for itself, which does not cover your binary.

**Fix.** Add an NSPrivacyAccessedAPICategoryFileTimestamp entry with reason C617.1 (files in the app container) — and consider adding the user-selected-file reason too, since MediaPrep.swift:146 can stat a document-picker URL. One plist edit fixes both platforms. Alternatively rewrite fileSize as `url.resourceValues(forKeys: [.fileSizeKey])`, but declare C617.1 either way rather than betting on which symbol the scanner keys off.

### A media message exists nowhere until every upload finishes, and nothing keeps the app alive to finish them — [#11](https://github.com/nettrash/family.connect/issues/11)
**Platform:** ios

**What.** ChatSyncCoordinator.sendMedia (ChatSyncCoordinator.swift:1420-1480) runs the whole upload loop and only enqueues the pending row at :1480; text takes the opposite order (:1294-1296), which is why sweepOutbox() can re-send a text message after a relaunch but has nothing to find for media. ConversationView.swift:1769-1771 clears the composer before the Task starts. There is no beginBackgroundTask and no background URLSession (grep returns nothing; UIBackgroundModes is audio+voip only). Backgrounding does not tear the view down, so the common case shows "Couldn't send that" and restores the set — but background AND navigate away and the caption, reply target and staged files are gone silently.

**Fix.** Wrap the upload in UIApplication.beginBackgroundTask (or move attachment uploads to a background URLSessionConfiguration), and stash the staged items alongside the draft in ConversationView's onDisappear. Note that simply enqueueing the row first is NOT safe: deliver(localID:) reads attachment ids off the row and would ship an empty bubble.

### WebSocket backoff resets on handshake, so a proxy that accepts and drops produces a reconnect storm — [#12](https://github.com/nettrash/family.connect/issues/12)
**Platform:** both

**What.** ChatSocket.swift:179 calls `backoff.reset()` as soon as the handshake ping succeeds, before the connection has proven durable. If receiveLoop throws immediately, the next delay is random(0...1)s forever — the 30s cap is never reached. Each .connected yield triggers ChatSyncCoordinator's full resync (isResyncing serializes them, so the REST load is back-to-back sweeps rather than 2/s, but the connect churn and the "Connecting…" banner flap at ~2Hz). The server itself has an accept-then-close path: registry.rs:141-151 kicks a connection whose send queue overflows with code 1001.

**Fix.** Reset the backoff only after the connection proves durable — record connectedAt at handshake and call backoff.reset() from receiveLoop on the first decoded frame, or at teardown if the connection lasted >10s. Leave the pre-connect resets in start()/resume() alone. Android's ChatSocketManager.kt:120-137 has the identical bug.

### A SwiftData container failure is a permanent dead end, and on macOS the advice it gives is wrong — [#13](https://github.com/nettrash/family.connect/issues/13)
**Platform:** both

**What.** FamilyConnectApp.swift:86 constructs the ModelContainer with no delete-and-retry; on failure every scene renders StoreErrorView (:364), a static view whose only guidance (:382) is "Reinstall Family Connect to start fresh." On macOS the app is sandboxed, so its store lives in ~/Library/Containers/me.nettrash.FamilyConnect/ and deleting the app does not clear it — the one escape hatch offered does not work on that platform.

**Fix.** On a container failure, delete the store files at configuration.url and retry ModelContainer once before falling back to StoreErrorView; keep the error view only for the second failure. The cache is 100% rebuildable from the server, which the error copy itself says. Fix the macOS wording too (it is a localized string in all 9 languages, so changing it later costs a translation pass).

### A refused CallKit transaction leaves the in-app End button inert with the microphone live — [#14](https://github.com/nettrash/family.connect/issues/14)
**Platform:** ios

**What.** CallKitController.swift:144-149 logs and discards every CXTransaction error with no completion, and CallManager.hangUp() (:363-369) routes exclusively through it on iOS. If CXStartCallAction is refused (another app owns a system call, a just-reset provider, a carrier call), no system UI exists and the in-app Hang Up does nothing. Worse, once media connects the guard is cancelled outright (CallManager.swift:1040-1041), so there is no 30/45/90s backstop at all — the call stays up until the far side hangs up or the app is force-quit, with the mic force-activated by ensureAudioRunning (WebRTCClient.swift:373-391). declineIncoming does not go through the bridge, so the ringing side always has a working exit; the trapped case is outgoing.

**Fix.** Give request(_:) a completion (or an onTransactionFailed closure on CallSystemBridge) and, on error, fall back to performHangUp(reportToSystem: false) / performAccept() so the app's own state machine still moves when the system refuses.

### Denied microphone permission makes "Record Audio" silently do nothing — [#15](https://github.com/nettrash/family.connect/issues/15)
**Platform:** both

**What.** AudioRecorder sets `failed = true` when requestPermission() returns false, but neither ConversationView.swift nor MacConversationView.swift ever reads recorder.failed — grep returns zero hits. A user who has denied the mic taps Record Audio and gets no recording bar, no alert, no explanation, on both platforms.

**Fix.** Read the flag at the two call sites and surface an alert pointing at Settings, the way the location-denied path already does (ConversationView.swift:2295).

### Location acquire path: the accuracy gate is dead code and there is no staleness check on iOS — [#16](https://github.com/nettrash/family.connect/issues/16)
**Platform:** both

**What.** LocationProvider.swift:148 reads `guard accuracy <= Self.goodEnoughMetres || !self.waiters.isEmpty else { return }` — waiters is non-empty for the entire duration of any wait (appended at :108 before startUpdatingLocation at :111), so the 100m bar never rejects anything and the first delivery CoreLocation makes, including the cached last-known fix it hands over immediately, is accepted and sent. iOS has no timestamp check at all; Android's LocationProvider.kt:120-125 does (FRESH_ENOUGH_MS = 2 minutes). Neither is covered by any test.

**Fix.** Fix the disjunct so the accuracy bar actually applies, and add the same freshness rule iOS is missing but Android already has — sending a stale cached position into a family chat is the one location failure that matters.

### The production server has no rate limiting, no storage quota, no backups and no monitoring — [#17](https://github.com/nettrash/family.connect/issues/17)
**Platform:** server

**What.** server/src/app.rs applies exactly one layer (DefaultBodyLimit) and the shipped nginx conf has no limit_req/limit_conn; both unauthenticated argon2 endpoints (register, and login — which deliberately runs a full verification even for an unknown username to close a timing oracle) are wide open on a box whose address is baked into the shipped binary. Argon2::default() is 19 MiB per hash and main.rs uses a bare #[tokio::main] (512-thread blocking pool), so concurrent logins can allocate ~9.7 GiB and OOM the process; systemd sets no MemoryMax. LimitsConfig has per-attachment (100 MB) and per-message caps but no per-user or per-family storage quota and no free-space check, so register→create family→upload can fill the disk PostgreSQL lives on. There is no backup procedure anywhere in the repo (config.example.toml:147 warns that a DB dump is no longer a complete backup and stops there), and no uptime monitoring — /api/v1/healthz does a real SELECT 1 but nobody polls it.

**Fix.** Before the box is public: add `limit_req` on /api/v1/auth/login and /register plus a looser one on /api/v1/, and ship the hardened conf in the repo (the new exact-match location blocks must repeat proxy_pass and the proxy_set_headers — nginx locations do not inherit from a sibling). Put the attachments directory on its own filesystem or LVM volume so a fill cannot take PostgreSQL down. Add a nightly pg_dump -Fc plus rsync/restic of /var/lib/family-connect/attachments to off-box storage, and test the restore once. Add external uptime monitoring on /api/v1/healthz with alerting somewhere you actually read, since your review notes promise Apple that server stays up.

### Verify the deployed server config before submitting: TURN wiring, APNs environment, retention, and [ai] — [#18](https://github.com/nettrash/family.connect/issues/18)
**Platform:** server

**What.** Four production values decide whether shipped features work, and none of them is in the repo. (1) coturn 4.6.1 IS deployed and healthy on fc.nettrash.me (STUN answers on UDP and TCP 3478, Allocate returns 401 with realm fc.nettrash.me, TLS 5349 serves the same Let's Encrypt cert) — but handlers_call.rs:115 only emits a TURN entry when [calls] turn_urls is non-empty, and a turn_secret that does not byte-match coturn's static-auth-secret fails silently at allocate time. (2) If [push.apns] environment is "sandbox", every alert push to a TestFlight/App Store build gets BadDeviceToken, which this code treats as permanently dead and DELETES the device row (events.rs:482) — and PushRegistrar only re-POSTs when the OS token changes, so it never recovers without a reinstall. (3) retention_days defaults to 100 and silently deletes messages and their photos/videos server-side. (4) [ai] enabled decides whether family message text goes to Azure OpenAI.

**Fix.** On the box: `grep -A8 '\[calls\]' /etc/family-connect/config.toml` (confirm turn_urls names the local coturn and turn_secret matches static-auth-secret), confirm `environment = "production"` and `bundle_id = "me.nettrash.FamilyConnect"` under [push.apns], decide retention_days deliberately (set 0 explicitly if you mean "keep everything"), and check whether [ai] is filled in. Then place one real relayed call and send one real alert push and one VoIP push to a TestFlight build before submitting. Also add a certbot deploy hook that signals coturn on renewal — the cert on 5349 expires 2026-11-17 and nothing currently reloads coturn.

### CHANGELOG and README describe a different, much smaller product — [#19](https://github.com/nettrash/family.connect/issues/19)
**Platform:** process

**What.** CHANGELOG has never been touched since the first commit (`git log -- CHANGELOG` returns only `18544d3 init`): its single v0.1.0 entry describes a text-only chat with "push-notification hooks without a transport yet", while the binary says 1.0 and 26 further migrations and 76 commits of features have shipped. server/Cargo.toml still says version = "0.1.0" and there are no git tags at all. README.md:28-30 says "Push notifications are a hook in v1... nothing is sent until an APNs/FCM transport is configured in a later version" while server/src/push.rs is 863 lines of working APNs + PushKit VoIP + FCM; README.md:5-7 says "Text messages in v1"; README.md:17 calls ios/ "SwiftUI client, iOS 17+" with no mention of the Mac app; README.md:3-4 claims "no third party ever sees a message", which an operator can falsify by filling in one config section.

**Fix.** Add a v1.0 CHANGELOG entry covering the real feature set, bump server/Cargo.toml, and correct README's push, feature-set, macOS and third-party-assistant claims. This is the repo's public face for self-hosters and takes under an hour.

### No CI: the 239 server integration tests and both client suites only run when someone remembers — [#20](https://github.com/nettrash/family.connect/issues/20)
**Platform:** process

**What.** .github/ contains no workflows (only modernize/java-upgrade hook scripts), and there is no fastlane, Makefile or git hook anywhere. Every one of the 239 server integration tests is #[ignore]d behind a live PostgreSQL, so the default `cargo test` exits 0 after running 190 unit tests while skipping the only coverage that exists for /me/delete, /auth/*, /devices, /attachments and /calls/ice. They do all pass — a full `cargo test -- --ignored` against a throwaway postgres:16 container is 239/239 green in ~1 minute at HEAD.

**Fix.** Add one GitHub Actions workflow: cargo fmt --check + clippy -D warnings + `cargo test -- --include-ignored` with a `services: postgres:16` container, and `xcodebuild test -only-testing:FamilyConnectTests` on macos-latest with CODE_SIGNING_ALLOWED=NO. Do not pin `name=iPhone 16 Pro` — it does not resolve on this machine; pin an explicit OS or a udid. Note the schemes' agvtool bump hangs off ArchiveAction only, so a test job will not dirty the checkout.

### The iPad build is an unadapted, stretched iPhone layout — [#21](https://github.com/nettrash/family.connect/issues/21)
**Platform:** ios

**What.** The binary declares UIDeviceFamily [1, 2] and all four iPad orientations, but there is no size-class adaptation anywhere: a repo-wide grep for horizontalSizeClass / userInterfaceIdiom returns zero hits, PlatformStyle.setupColumn() is literally `self` on iOS, and the only NavigationSplitView is the Mac one. A 13-inch iPad shows an edge-to-edge sign-in form with ~60% of the screen empty, and text balloons capped only by a 48pt spacer (~1300pt wide in landscape) sitting beside 240pt-capped photo bubbles.

**Fix.** Either adapt: clamp setupColumn() on regular width, move ChatListView to a NavigationSplitView mirroring MacChatView, and cap the text balloon at ~0.75 of the container. Or set TARGETED_DEVICE_FAMILY = "1" for 1.0 — the app still runs on iPad in compatibility mode and the iPad screenshot slot disappears.

### Nothing in either scheme or checklist guards against archiving with the wrong scheme — [#22](https://github.com/nettrash/family.connect/issues/22)
**Platform:** process

**What.** FamilyConnect.xcscheme archives with "Release" (FC_DEFAULT_SERVER_URL empty); only FamilyConnect-nettrash.xcscheme archives with "Release-nettrash", which is what sets https://fc.nettrash.me. README.md:65-69 documents the correct command, but the appstore.md pre-submission checklist never mentions it, and the plainly-named scheme is the wrong one. A Release archive would land the reviewer on the "Server address" screen, contradicting review-notes step 1 (they could recover, since the notes print the URL, but it reads as a broken build).

**Fix.** Add a checklist line: archive only from FamilyConnect-nettrash, and before uploading run `plutil -extract FCDefaultServerURL raw <archive>/Products/Applications/FamilyConnect.app/Info.plist` and confirm it prints https://fc.nettrash.me.

### No age-rating answers recorded, and no macOS section, in the pre-submission checklist — [#23](https://github.com/nettrash/family.connect/issues/23)
**Platform:** process

**What.** ios/docs/appstore.md has no Age Rating section at all (zero hits for "age"/"rating"), even though the app is unmoderated person-to-person messaging with media plus an AI assistant that writes into the shared family chat. It also has zero occurrences of macOS/Mac — no Mac description, no Mac screenshots plan, no Mac review notes — and README's store-build section documents only an iOS archive and an Android bundle, with no macOS archive step anywhere in the repo.

**Fix.** Add an Age Rating section recording the answers actually given (user-generated content: yes; in-app messaging: yes; in-app chatbot: yes if the assistant is enabled) and the resulting band, and do NOT select the Kids Category. Add a macOS section (or a sibling appstore-macos.md) if the Mac app ships, plus the macOS archive command to README. Note the Mac build shares the bundle id, so it is a macOS platform added to the same App Store record, not a second record.

---
## Nice to have (14)

- [#24](https://github.com/nettrash/family.connect/issues/24) Quiet the 24 default-actor-isolation warnings in the Release build (RingbackTone 10, WebRTCClient 4, LinkPreviewLoader 2, plus ShareImport/AttachmentViewer/AudioPlayerView) by marking the relevant helpers `nonisolated` — Swift 6 migration debt, not a runtime hazard. The two RemoteFirstFrameRelay warnings in particular are reads of immutable `let`s and carry no data race.
- [#25](https://github.com/nettrash/family.connect/issues/25) Drop the unused `photoLibrary: .shared()` argument at SettingsView.swift:152 and ConversationView.swift:1136 so the binary stops referencing PHPhotoLibrary — behaviour-neutral, since nothing uses PHAsset or itemIdentifier. Do NOT add NSPhotoLibraryUsageDescription; the app genuinely never reads the library.
- [#26](https://github.com/nettrash/family.connect/issues/26) Fix MediaPrep.swift:468's no-op `as? Int` on an already-Int? expression.
- [#27](https://github.com/nettrash/family.connect/issues/27) Add a Settings scene to the Mac app (`Settings { MacSettingsView() }`) so ⌘, and App menu → Settings… work and the panel becomes a resizable window instead of a fixed 460x530 sheet reachable only from the main window.
- [#28](https://github.com/nettrash/family.connect/issues/28) Add drag-and-drop of files into a Mac conversation (.dropDestination(for: URL.self) feeding MediaPrep.prepare) and .draggable on attachment tiles — the read-only entitlement already covers a dropped file.
- [#29](https://github.com/nettrash/family.connect/issues/29) Enable ENABLE_HARDENED_RUNTIME for macOS (optional for the Mac App Store, mandatory only for Developer ID notarization) — every sibling Mac app in the workspace sets it.
- [#30](https://github.com/nettrash/family.connect/issues/30) Localize the five permission usage strings: they are English-only INFOPLIST_KEY_* literals in a bundle that ships 9 fully translated languages, on both platforms. Add an InfoPlist.xcstrings.
- [#31](https://github.com/nettrash/family.connect/issues/31) Add a macOS launch smoke test (add macosx to FamilyConnectUITests' SUPPORTED_PLATFORMS, or a Mac-only scheme) — the macOS test lane also flakes roughly 1-in-4 with "the test runner hung before establishing connection".
- [#32](https://github.com/nettrash/family.connect/issues/32) Scope the iOS-only INFOPLIST_KEY_* settings (scene manifest, launch screen, orientations, indirect input) with [sdk=iphone*] so they stop leaking into the macOS Info.plist, and add INFOPLIST_KEY_NSHumanReadableCopyright so the Mac About panel is not blank.
- [#33](https://github.com/nettrash/family.connect/issues/33) Add tests for the untested pure logic: ServerURLNormalizer (which currently rejects bare LAN hostnames like `http://nas` and IPv6 literals like `http://[fd00::1]` despite claiming to mirror the ATS local-networking exception), AudioRecorder.timeLabel, and a share-extension producer→consumer round-trip.
- [#34](https://github.com/nettrash/family.connect/issues/34) Hoist the four hand-off constants the Share Extension re-declares privately (scheme, host, app group, inbox folder) into one dependency-free file shared by both targets, or pin them with a test.
- [#35](https://github.com/nettrash/family.connect/issues/35) Clean up orphaned Share Extension staging directories in the App Group container when a hand-off is never completed.
- [#36](https://github.com/nettrash/family.connect/issues/36) Wrap the `--uitest-reset` launch argument (which wipes the keychain token and all defaults) in #if DEBUG — it currently ships in Release.
- [#37](https://github.com/nettrash/family.connect/issues/37) Decide the App Store name: the listing says "Family Connect", the home screen says "Family". Either is defensible (the on-device name is the leading word of the store name, which is what the sibling apps already do), but reserve the name before anything else and pre-agree a fallback.

---
## Critical path
TONIGHT (2-3 hours, all independent of everything else): (1) Rewrite ios/docs/appstore.md end to end — Description, Keywords, Promotional Text, Beta App Description, What to Test and the App Review notes — against the real feature set, deleting the stale account-deletion blockquote, the "text messages only" lines (:31, :66, :80) and the "no third-party SDKs" claims (:31, :63), and fixing :60's wrong deletion semantics. (2) Provision the two demo accounts plus a seeded reviewer family on fc.nettrash.me and substitute all six placeholders. (3) Four one-line code/config fixes while you are in there: ITSAppUsesNonExemptEncryption = NO in both plists, com.apple.developer.aps-environment in FamilyConnect-macOS.entitlements, files.user-selected.read-write in the same file, and the FileTimestamp entry in PrivacyInfo.xcprivacy. (4) SSH to the box and check four config values: [calls] turn_urls/turn_secret against coturn, [push.apns] environment = production, retention_days, and whether [ai] is enabled — these four answers are inputs to the privacy policy you write tomorrow, so do them before you start writing.

DAY 1 — THE POLICY, WHICH GATES THE MOST: write and publish frontend/assets/appstore/familyconnect/{privacy,support}.html on nettrash.me (model them on scan/privacy.html), add the home.rs card, and add a Privacy Policy link row to SettingsView and MacSettingsView. Nothing downstream can start until this text exists: the App Store Connect record cannot be created without the URL, the App Privacy questionnaire answers are derived from the same decisions, and the age-rating answers ride along. Finish the day by filling the App Privacy questionnaire (the corrected 7-type Linked=true list, not appstore.md:101's two types) and mirroring it into PrivacyInfo.xcprivacy.

DAYS 1-3 IN PARALLEL — THE LONG POLE: **DONE 2026-08-30 — see the status update at the top.** Built on all three
platforms plus the server, and adversarially audited afterwards (five missed defects found and fixed).
The path below is left as written; what it describes now exists. Original text: build Report and Block. This is independent code and should start the same morning as the policy work, not after it. Minimum that clears Guideline 1.2's two code-only legs: a per-member block (server-side mute so a blocked member's messages and calls never arrive, plus client-side hide) reachable from the member roster and from a message, and a Report action on a message and on a member posting to a new endpoint. Both surfaces need the iOS and macOS menus. Budget 2-3 days including tests and the Android-side protocol note.

DAY 4 — AFTER THE UI IS FINAL: screenshots. They have to come last because adding Report/Block changes the menus you are photographing, and because you want the seeded demo family to be the content in frame. Capture the 6.9" iPhone set (iPhone 16 Pro Max or 16 Plus — NOT iPhone 16 Pro, which is the 6.3" class) and, if you keep TARGETED_DEVICE_FAMILY = "1,2", the 13" iPad set on iPad Pro 13-inch (M4). If you decide tonight to drop iPad, this is a half-day instead of a day.

DAY 4-5 — SUBMIT: create the App Store Connect record (reserve the name first — that is the one thing that can force a rename), fill Privacy Policy URL, Support URL, categories, copyright, content rights, age rating and export compliance, archive with the FamilyConnect-nettrash scheme ONLY, verify with `plutil -extract FCDefaultServerURL raw <archive>/.../Info.plist` before uploading, upload, and submit. Commit the agvtool build-number bump with the release commit rather than leaving the tree dirty.

REALISTIC CALENDAR TO A SUBMITTABLE iOS BUILD: 4-5 working days if you build Report and Block properly; 2-3 days if you ship a minimal client-side block plus a report-by-email and accept a higher chance of a 1.2 round trip. Everything except the 1.2 work is paperwork you can grind through in a long evening and a day.

macOS: do not put it on this path. Even after the two entitlement fixes it needs its own listing copy, its own Mac screenshot set, its own review notes, and a documented archive/export procedure that does not exist anywhere in the repo. Ship iOS, then give macOS its own week. If you insist on shipping both together, add 1-2 days and write the macOS section of appstore.md before you touch anything else.

SERVER WORK THAT SHOULD LAND BEFORE THE APP IS PUBLIC, not before submission: nginx limit_req on the two auth endpoints, a separate volume for the attachments directory, a nightly pg_dump + attachments backup with one tested restore, and uptime monitoring on /api/v1/healthz. None of this gates App Review; all of it gates the day a stranger downloads the app and finds your box.

---
## Open questions — decisions only nettrash can make

- Ship macOS in this release, or hold it? The Mac app is real and already TestFlight-distributed (build 100 carries a _MASReceipt), and it is genuinely well-built — but it has zero listing copy, zero Mac screenshots, no documented archive/export procedure, and two entitlement bugs. Holding it costs you nothing on the iOS timeline; shipping it together adds 1-2 days minimum.
- Keep TARGETED_DEVICE_FAMILY = "1,2"? Keeping iPad makes the 13" screenshot set mandatory and puts an unadapted stretched-iPhone layout on the store. Dropping to "1" removes both problems and the app still runs on iPad in compatibility mode. Adapting it properly (NavigationSplitView on regular width, balloon width cap) is a day of UI work you do not have to spend now.
- Keep open registration on fc.nettrash.me? Your own checklist (appstore.md:102) already flags this. Anyone who downloads the app can create an account and a family on your box, which is the fact that makes Guideline 1.2 bite hardest and is also an unbounded-storage exposure. Adding a [registration] switch and an invite gate would materially shrink both — but it also means the reviewer needs working credentials rather than being able to self-register, so the two decisions are coupled.
- Is [ai] enabled on fc.nettrash.me? If yes, family message text goes to Azure OpenAI, and because ai_history defaults to TRUE (migration 0019), a single @ai mention ships up to 30 days / 200 messages / 40,000 characters of OTHER members' words with display names and timestamps, with no in-app consent. That needs naming in the privacy policy, disclosing in the App Privacy answers, mentioning in the description, and arguably a consent step before the first mention. If it is disabled, say so explicitly in the review notes.
- What is retention_days on the live box? The default is 100, which silently and permanently deletes every family's messages AND their photos and videos older than ~3 months, server-side. Either set it to 0 explicitly (keep everything) and accept the storage growth, or keep 100 and state the number plainly in the privacy policy and the description. Whichever you choose, do not leave it inherited by accident.
- Export compliance: declare false under the standard exemption? The app implements no crypto of its own, but WebRTC bundles BoringSSL rather than using Apple's TLS, so this is not the trivially-exempt case your other apps are. Decide once, record the reasoning, and bake the key in.
- App Store name: is "Family Connect" free? Reserve it before anything else — a rename after you have written the listing copy and captured screenshots is the one late-breaking change that costs real time. Pre-agree the fallback ("nettrash Family Connect" matches your existing store slugs).
- How much server hardening before the box is public? Rate limiting, storage quotas, backups and monitoring are all missing. None of it gates App Review. All of it gates the first week after launch, when strangers with the App Store build know the address of a single unmonitored box holding other families' photos and conversations with no restore path.

---
## Appendix A — all 111 verified findings
Every finding that survived adversarial verification, grouped by the audit dimension that found
it. The blocker / should-fix / nice-to-have lists above are these, deduplicated and merged.

### `ios-build-test` (1)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F1 | Video-call renderer touches a @MainActor property from WebRTC's media thread (2 warnings) | both | nice-to-have |

**F1 — Video-call renderer touches a @MainActor property from WebRTC's media thread (2 warnings)**  
*both · nice-to-have*

> **Evidence.** ios-release.log: `FamilyConnect/Core/Calls/WebRTCClient.swift:589:9: warning: main actor-isolated property 'wrapped' can not be referenced from a nonisolated context` and the same at 593:9. Source: `nonisolated func setSize(_ size: CGSize) { wrapped.setSize(size) }` and `nonisolated func renderFrame(_ frame: RTCVideoFrame?) { wrapped.renderFrame(frame) ... }` — `wrapped` is stored on a type that SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor makes main-actor-isolated.

> **Fix.** Either mark the wrapper type `nonisolated` (or the `wrapped`/`onFirstFrame` storage `nonisolated(unsafe)` with a documented thread contract), or hop the forwarding calls through the renderer's own serial queue rather than reading main-actor state from the media thread. Same treatment for the two `reference to captured var 'self' in concurrently-executing code` warnings at WebRTCClient.swift:148:34 and :234:27.

### `macos-build-test` (2)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F7 | macOS entitlements use the iOS push key `aps-environment`, so the shipped Mac app has no push entitlement at all | macos | should-fix |
| F8 | The Mac app is invisible in the release documentation — README.md and ios/docs/appstore.md never mention macOS | macos | nice-to-have |

**F7 — macOS entitlements use the iOS push key `aps-environment`, so the shipped Mac app has no push entitlement at all**  
*macos · should-fix*

> **Evidence.** ios/FamilyConnect/FamilyConnect-macOS.entitlements declares `<key>aps-environment</key><string>development</string>` (with a comment saying Xcode will rewrite it to production on export). But macOS's key is `com.apple.developer.aps-environment` — both Mac provisioning profiles for this app grant exactly that name: `security cms -D -i 53a6806c-….provisionprofile | plutil -p` → "com.apple.developer.aps-environment" => "development", and the Mac Team STORE profile → "com.apple.developer.aps-environment" => "production". Because the file's key doesn't match, Xcode strips it: `codesign -d --entitlements - --xml <signed Release-nettrash>/FamilyConnect.app` lists app-sandbox, application-groups, network.client, network.server, device.camera, device.audio-input, personal-information.location, files.user-selected.read-only, get-task-allow — and NO aps key of any spelling. The same is true of the build already exported and installed at /Applications/FamilyConnect.app (CFBundleVersion 100, entitlements include beta-reports-active, i.e. an App Store/TestFlight export). Meanwhile the Mac build really does register: ios/FamilyConnect/Core/PushRegistrar.swift:190-194 `#elseif os(macOS) NSApplication.shared.registerForRemoteNotifications()`, PushRegistrar.swift:96 reports platform "macos" to POST /devices, and ios/FamilyConnect/AppDelegate.swift:20-21 states in comments "The Mac DOES register for push and has its own platform value (`macos`, migration 0013)".

> **Fix.** In ios/FamilyConnect/FamilyConnect-macOS.entitlements rename the key from `aps-environment` to `com.apple.developer.aps-environment` (keep the value "development"; Xcode rewrites it to production on App Store export, exactly as the existing comment describes). Leave ios/FamilyConnect/FamilyConnect.entitlements alone — `aps-environment` is correct for iOS. Verify with `codesign -d --entitlements - --xml <built app>` that com.apple.developer.aps-environment now survives signing, then re-check that a Mac run reaches PushRegistrar.handleDeviceToken instead of MacAppDelegate's failure callback.

**F8 — The Mac app is invisible in the release documentation — README.md and ios/docs/appstore.md never mention macOS**  
*macos · nice-to-have*

> **Evidence.** `grep -n -i "macos|mac app|notariz|developer id" README.md ios/docs/appstore.md` returns zero hits in both files. README.md:17 describes the client as "ios/  # SwiftUI client, iOS 17+ (FamilyConnect.xcodeproj)"; its test snippet (README.md:52-55) only uses `-destination 'platform=iOS Simulator,name=iPhone 16'` and its archive snippet (README.md:65-68) only archives for iOS. ios/docs/appstore.md's headings (Promotional Text, Description, Keywords, Notes for App Review, Beta App Description, What to Test, Pre-submission checklist) are all iOS-only. Yet the Mac app demonstrably exists and is intended to ship: it is installed and running at /Applications/FamilyConnect.app (pid 1175 during this audit), it was exported with an App Store profile (beta-reports-active in its entitlements), and both "Mac Team Store Provisioning Profile: me.nettrash.FamilyConnect" and "…ShareExtension" exist in ~/Library/Developer/Xcode/UserData/Provisioning Profiles/.

> **Fix.** Add a macOS section to ios/docs/appstore.md (Mac App Store listing copy, Mac screenshot set, and review notes covering the same demo-server access as iOS) and add the macOS commands to README.md next to the iOS ones: `xcodebuild test -project FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=macOS' -only-testing:FamilyConnectTests CODE_SIGNING_ALLOWED=NO` and `xcodebuild archive -scheme FamilyConnect-nettrash -destination 'generic/platform=macOS'`. Also update README.md:17's "iOS 17+" description to say the one target ships iOS 17+ and macOS 14+.

### `archive-validate` (3)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F14 | macOS build ships with NO push entitlement — the entitlements file uses the iOS key spelling, so Xcode drops it | macos | should-fix |
| F15 | Saving an attachment on macOS writes to an NSSavePanel destination the sandbox only grants read access to — and the failure is swallowed | macos | should-fix |
| F16 | ITSAppUsesNonExemptEncryption is absent from both Info.plists, so every upload stalls on the export-compliance question | both | should-fix |

**F14 — macOS build ships with NO push entitlement — the entitlements file uses the iOS key spelling, so Xcode drops it**  
*macos · should-fix*

> **Evidence.** ios/FamilyConnect/FamilyConnect-macOS.entitlements:25 declares `<key>aps-environment</key><string>development</string>`. The entitlements Xcode actually generated and signed with — .../dd-arch-mac/Build/Intermediates.noindex/ArchiveIntermediates/FamilyConnect-nettrash/IntermediateBuildFilesPath/FamilyConnect.build/Release-nettrash/FamilyConnect.build/FamilyConnect.app.xcent — contains no push key of any kind: only com.apple.application-identifier, com.apple.developer.team-identifier, app-sandbox, application-groups, device.audio-input, device.camera, files.user-selected.read-only, network.client, network.server, personal-information.location. `codesign -d --entitlements :-` on the archived .app confirms the same set. Meanwhile the Mac Team Provisioning Profile embedded in that same .app grants `com.apple.developer.aps-environment => development` — the macOS spelling. Xcode filters the requested entitlements against the profile, `aps-environment` is not in the macOS profile, so it was silently dropped. The app does register: ios/FamilyConnect/Core/PushRegistrar.swift:193 calls `NSApplication.shared.registerForRemoteNotifications()`.

> **Fix.** In ios/FamilyConnect/FamilyConnect-macOS.entitlements, rename the key from `aps-environment` to `com.apple.developer.aps-environment` (macOS uses the com.apple.developer prefix; iOS does not — leave FamilyConnect.entitlements:11 alone). Re-archive and confirm the generated FamilyConnect.app.xcent now carries the key, then verify on a signed Mac build that PushRegistrar logs a token instead of a didFailToRegister error.

**F15 — Saving an attachment on macOS writes to an NSSavePanel destination the sandbox only grants read access to — and the failure is swallowed**  
*macos · should-fix*

> **Evidence.** ios/FamilyConnect/MacViews/MacAttachmentViewer.swift:160 opens `NSSavePanel()`, and lines 166-167 write to the chosen URL: `try? FileManager.default.removeItem(at: destination)` / `try? FileManager.default.copyItem(at: source, to: destination)`. The signed macOS entitlements (from `codesign -d --entitlements :-` on the archived app, and ios/FamilyConnect/FamilyConnect-macOS.entitlements) grant `com.apple.security.files.user-selected.read-only` and no read-write variant. Per Apple's entitlement documentation, `files.user-selected.read-only` grants read-only access to files chosen through an Open OR Save dialog; write access to a Save-panel destination requires `com.apple.security.files.user-selected.read-write`.

> **Fix.** Add `<key>com.apple.security.files.user-selected.read-write</key><true/>` to ios/FamilyConnect/FamilyConnect-macOS.entitlements (it supersedes the read-only key, which can then be removed), and while there, stop swallowing the failure — replace the `try?` on the copy with a real catch that surfaces an alert, so a future sandbox denial is visible instead of silent.

**F16 — ITSAppUsesNonExemptEncryption is absent from both Info.plists, so every upload stalls on the export-compliance question**  
*both · should-fix*

> **Evidence.** `plutil -p` of the archived iOS Info.plist and the archived macOS Contents/Info.plist shows no ITSAppUsesNonExemptEncryption key. `grep -rn 'ITSAppUsesNonExemptEncryption\|NonExempt' FamilyConnect.xcodeproj/project.pbxproj FamilyConnect/Info.plist FamilyConnect/Info-macOS.plist docs/appstore.md` returns zero hits — the key is nowhere in the project, and the App Store doc does not mention export compliance at all.

> **Fix.** Decide the correct answer for this app's actual crypto (HTTPS/TLS only is the standard exemption; the app is described as end-to-end/self-hosted, so check what it actually does before answering) and then add the key to both Info.plists — ios/FamilyConnect/Info.plist and ios/FamilyConnect/Info-macOS.plist — as `<key>ITSAppUsesNonExemptEncryption</key><false/>` if it qualifies as exempt, or `<true/>` plus `ITSEncryptionExportComplianceCode` once you hold a code. Record the reasoning in ios/docs/appstore.md so it survives the next release.

### `plists-entitlements` (5)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F21 | macOS App Group id is not Team-ID-prefixed — sandboxed container APIs hang forever, breaking the Mac share extension | macos | nice-to-have |
| F22 | macOS entitlements use the iOS key `aps-environment`; Xcode silently strips it, so the Mac build ships with no push entitlement | macos | should-fix |
| F24 | ITSAppUsesNonExemptEncryption is absent from both Info.plists | both | should-fix |
| F25 | macOS "Save" writes through NSSavePanel but the sandbox only grants files.user-selected.read-only — the save silently does nothing | macos | should-fix |
| F26 | No NSPhotoLibraryUsageDescription while the binary references PHPhotoLibrary — ITMS-90683 risk, and the declared Add string contradicts the code | ios | nice-to-have |

**F21 — macOS App Group id is not Team-ID-prefixed — sandboxed container APIs hang forever, breaking the Mac share extension**  
*macos · nice-to-have*

> **Evidence.** Entitlements use the bare iOS form on macOS: /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/FamilyConnect-macOS.entitlements:61-63 and /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnectShareExtension/FamilyConnectShareExtension.entitlements:18-21 both contain `<string>group.me.nettrash.FamilyConnect</string>`. Swift hardcodes the same bare string: /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Core/ShareImport.swift:32 (`static let appGroup = "group.me.nettrash.FamilyConnect"`), consumed at /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Core/AppSession.swift:184-185 (default arg `FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: ShareImport.appGroup)`), and /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnectShareExtension/ShareViewController.swift:43 + :187-189.

EMPIRICAL A/B (two identical ad-hoc .app bundles, same Apple Development identity `A332C69948E33B7C7D6C6CDDC8468CF5F79B5A75`, same binary, both with `com.apple.security.app-sandbox`, differing ONLY in the group-id form, on a group id that had never existed):
  bare: STILL RUNNING (HUNG)
  --- bare output:
  nbare[27591] PROBE start group.me.nettrash.ProbeXYZ
  nbare[27591] PROBE url /Users/nettrash/Library/Group Containers/group.me.nettrash.ProbeXYZ
  (no further output; still alive after 20 s; killed with SIGKILL)
  pref: exited
  --- pref output:
  npref[27593] PROBE start V4WM2SJ8Q9.group.me.nettrash.ProbeXYZ
  npref[27593] PROBE url /Users/nettrash/Library/Group Containers/V4WM2SJ8Q9.group.me.nettrash.ProbeXYZ
  npref[27593] PROBE WRITE OK   (elapsed ~1 ms)

A second sandboxed probe entitled to the team-prefixed id was additionally DENIED the bare path: `group.me.nettrash.FamilyConnect: WRITE FAILED — NSCocoaErrorDomain Code=513 … NSPOSIXErrorDomain Code=1 "Operation not permitted"` while `V4WM2SJ8Q9.group.me.nettrash.FamilyConnect: WRITE OK`.

The macOS main-app provisioning profile Xcode fetched does not even carry the entitlement — `security cms -D -i .../FamilyConnect.app/Contents/embedded.provisionprofile` Entitlements = { com.apple.application-identifier, com.apple.developer.aps-environment, com.apple.developer.team-identifier, keychain-access-groups } with NO `com.apple.security.application-groups` (the iOS profile does have it).

> **Fix.** On macOS the App Group identifier must be Team-ID-prefixed. (1) Change FamilyConnect-macOS.entitlements:63 and FamilyConnectShareExtension.entitlements:20 to `V4WM2SJ8Q9.group.me.nettrash.FamilyConnect` — note the share extension needs a SEPARATE entitlements file per platform (via `CODE_SIGN_ENTITLEMENTS[sdk=macosx*]` on the FamilyConnectShareExtension target, as the app already does) because iOS must keep the bare form. (2) Make the Swift constant platform-conditional: `#if os(macOS) static let appGroup = "V4WM2SJ8Q9.group.me.nettrash.FamilyConnect" #else static let appGroup = "group.me.nettrash.FamilyConnect" #endif` in ShareImport.swift:32, and mirror it in ShareViewController.swift:43. (3) Re-enable App Groups on the macOS side of App ID `me.nettrash.FamilyConnect` in the developer portal so the Mac profile regenerates with the entitlement (see finding `macos-appid-missing-app-groups`).

**F22 — macOS entitlements use the iOS key `aps-environment`; Xcode silently strips it, so the Mac build ships with no push entitlement**  
*macos · should-fix*

> **Evidence.** /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/FamilyConnect-macOS.entitlements:25 declares `<key>aps-environment</key><string>development</string>`. The macOS entitlement key is `com.apple.developer.aps-environment`, and the Mac provisioning profile Xcode fetched carries exactly that: profile Entitlements include `"com.apple.developer.aps-environment" => "development"`. Because the source key does not match, Xcode's ProcessProductPackaging step drops it — the build log for target 'FamilyConnect' prints the resulting entitlement set and it has NO push key at all:
  Entitlements:
  { com.apple.application-identifier; com.apple.developer.team-identifier; com.apple.security.app-sandbox; com.apple.security.application-groups; com.apple.security.device.audio-input; com.apple.security.device.camera; com.apple.security.files.user-selected.read-only; com.apple.security.get-task-allow; com.apple.security.network.client; com.apple.security.network.server; com.apple.security.personal-information.location }
`codesign -d --entitlements` on the built .app confirms the same 11 keys and no aps-environment. The iOS build, by contrast, signs with `"aps-environment" => "development"` present.

This has already shipped: `codesign -d --entitlements - /Applications/FamilyConnect.app` (a distribution-signed build — it carries `"beta-reports-active" => true`) also has NO aps-environment.

The Mac app really does register: /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Core/PushRegistrar.swift:192-193 `#elseif os(macOS)` → `NSApplication.shared.registerForRemoteNotifications()`.

> **Fix.** In /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/FamilyConnect-macOS.entitlements, rename the key on line 25 from `aps-environment` to `com.apple.developer.aps-environment` (keep the value `development`; Xcode still rewrites it to `production` on export). Then rebuild and confirm with `codesign -d --entitlements - --xml <built .app> | plutil -p -` that the key survives packaging.

**F24 — ITSAppUsesNonExemptEncryption is absent from both Info.plists**  
*both · should-fix*

> **Evidence.** `grep -rn "ITSAppUsesNonExemptEncryption\|NonExempt" /Users/nettrash/Develop/nettrash.me/family.connect` returns zero hits, and neither built Info.plist contains it: `plutil -p <derived>/Release-iphoneos/FamilyConnect.app/Info.plist` and `.../Release/FamilyConnect.app/Contents/Info.plist` — no ITSApp* key in either. The app does use encryption: HTTPS everywhere plus WebRTC's DTLS-SRTP, and it bundles a third-party crypto implementation — `WebRTC.framework` (stasel/WebRTC 151.0.0) is embedded in both bundles (`<app>/Frameworks/WebRTC.framework` on iOS, `Contents/Frameworks/WebRTC.framework/Versions/A` on macOS).

> **Fix.** Add `INFOPLIST_KEY_ITSAppUsesNonExemptEncryption` to the FamilyConnect target's build settings so it lands in both platforms' plists, and set it deliberately: if the intent is to claim the standard exemption (HTTPS/TLS + DTLS-SRTP only, no proprietary crypto), set it to NO and keep a note of the reasoning; if instead you declare non-exempt use, set YES and add `ITSEncryptionExportComplianceCode` with the code from the CCATS/self-classification. Because BoringSSL ships inside WebRTC.framework rather than coming from Apple's OS, confirm the annual self-classification report obligation applies before choosing NO.

**F25 — macOS "Save" writes through NSSavePanel but the sandbox only grants files.user-selected.read-only — the save silently does nothing**  
*macos · should-fix*

> **Evidence.** /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/FamilyConnect-macOS.entitlements:37 grants only `com.apple.security.files.user-selected.read-only` (confirmed in the signed binary: `codesign -d --entitlements` shows `com.apple.security.files.user-selected.read-only => true` and no read-write variant). But /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/MacViews/MacAttachmentViewer.swift:160-168 opens an NSSavePanel and WRITES to the chosen destination:
    let panel = NSSavePanel()
    guard panel.runModal() == .OK, let destination = panel.url else { return }
    try? FileManager.default.removeItem(at: destination)
    try? FileManager.default.copyItem(at: source, to: destination)
Both writes are `try?`, so the failure is swallowed with no error surfaced. The entitlements file's own comment (lines 14-19) asserts "anything leaving it goes through the share sheet, which needs no entitlement of its own" — that is contradicted by this NSSavePanel path.

> **Fix.** Change FamilyConnect-macOS.entitlements:37 from `com.apple.security.files.user-selected.read-only` to `com.apple.security.files.user-selected.read-write` (it subsumes read, so MacFilePicker's NSOpenPanel keeps working), update the surrounding comment, and stop swallowing the errors in MacAttachmentViewer.save() — surface a failure to the user instead of `try?` so a future sandbox denial is visible.

**F26 — No NSPhotoLibraryUsageDescription while the binary references PHPhotoLibrary — ITMS-90683 risk, and the declared Add string contradicts the code**  
*ios · nice-to-have*

> **Evidence.** Only the write key is declared — project.pbxproj lines 550 / 603 / 812: `INFOPLIST_KEY_NSPhotoLibraryAddUsageDescription = "…Family never reads your library — the picker hands over only what you choose."` with no `INFOPLIST_KEY_NSPhotoLibraryUsageDescription` anywhere. Two call sites pass a live PHPhotoLibrary into the picker: /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Views/SettingsView.swift:152 `PhotosPicker(selection: $pickedPhoto, matching: .images, photoLibrary: .shared())` and /Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Views/ConversationView.swift:1131-1136 `.photosPicker(… photoLibrary: .shared())`. That leaves the class in the shipped binary: `nm -u <derived>/Release-iphoneos/FamilyConnect.app/FamilyConnect | grep PHPhotoLibrary` → `_OBJC_CLASS_$_PHPhotoLibrary`, and `otool -L` shows Photos.framework and PhotosUI.framework linked.

I verified the RUNTIME is fine: I built a minimal iOS 17 simulator app with NO NSPhotoLibraryUsageDescription that calls `PHPhotoLibrary.shared()` and presents `PHPickerViewController(configuration: PHPickerConfiguration(photoLibrary:))`. It did not crash — log: step1 → step2 (`shared()` returned) → step4 (presenting) → step5 (presented OK) → step6 (alive 6 s later) — and a simulator screenshot shows the picker rendering the library with the banner "Private Access to Photos — Your photo library is shown here, but \"PPTest\" can only access the items you select." So this is NOT a crash-on-first-use and not a functional break.

> **Fix.** Either drop the `photoLibrary: .shared()` argument at SettingsView.swift:152 and ConversationView.swift:1136 (the app only ever calls `loadTransferable`, never resolves PHAssets, so the plain out-of-process picker is sufficient and removes the PHPhotoLibrary reference entirely — the cleaner fix), or add `INFOPLIST_KEY_NSPhotoLibraryUsageDescription` to all three build configurations and reword the Add string so it no longer claims the library is never read.

### `privacy-manifest` (9)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F28 | App Privacy label / privacy manifest declare zero data collection, but the App Store build ships pointed at a developer-operated server that stores everything | both | should-fix |
| F29 | attributesOfItemAtPath: is compiled into the app binary but NSPrivacyAccessedAPICategoryFileTimestamp is not declared | both | should-fix |
| F30 | appstore.md instructs "Data Not Linked to You", which is wrong for an app with server-side accounts | both | should-fix |
| F31 | No privacy policy or support page exists for Family Connect; App Store Connect will not accept the submission without a Privacy Policy URL | both | blocker |
| F32 | PrivacyInfo.xcprivacy's NSPrivacyCollectedDataTypes stays an empty array even after the label is corrected | both | nice-to-have |
| F33 | "No third-party SDKs of any kind" is stated in both the App Store description and the review notes while WebRTC.framework ships inside the app | both | should-fix |
| F34 | Calls default to Google's public STUN server, so the app contacts a host the family does not own on every call — undisclosed in the description, review notes and privacy story | both | nice-to-have |
| F35 | Link previews (arbitrary web hosts) and map previews (Apple) are ON by default and are not mentioned in the store copy or review notes | both | nice-to-have |
| F36 | The server can forward member message text to Azure OpenAI; whether fc.nettrash.me has it enabled is unverified and undisclosed | server | should-fix |

**F28 — App Privacy label / privacy manifest declare zero data collection, but the App Store build ships pointed at a developer-operated server that stores everything**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/PrivacyInfo.xcprivacy: `<key>NSPrivacyCollectedDataTypes</key><array/>` (empty). ios/FamilyConnect.xcodeproj/project.pbxproj:769 (Release-nettrash) `FC_DEFAULT_SERVER_URL = "https://fc.nettrash.me";` vs :438 and :507 (Debug/Release) `FC_DEFAULT_SERVER_URL = "";`. ios/FamilyConnect/Info.plist:56-57 carries it as FCDefaultServerURL; ios/FamilyConnect/Storage/AppSettings.swift:71-83 `defaultServerURL` adopts it on first run so store users never see the server-setup screen. README.md:159: "Messages are stored in plaintext in *your* PostgreSQL". docs/protocol.md:63-72 (User/Member carry `username`, `display_name`, `avatar_version`), :788-801 (`POST /attachments?kind=location&latitude=…&longitude=…&accuracy_m=…`, range-checked WGS 84 degrees), :1215 (photo/video/audio/file/location attachment upload), :1253 (`POST /devices {platform, push_token, voip_token}`).

> **Fix.** Fill the App Store Connect App Privacy questionnaire and mirror it in NSPrivacyCollectedDataTypes. Declare, all with purpose App Functionality only, NSPrivacyCollectedDataTypeTracking=false, NSPrivacyCollectedDataTypeLinked=true: NSPrivacyCollectedDataTypeOtherUserContent (message text, polls, reactions, board notes), NSPrivacyCollectedDataTypePhotosorVideos (chat media + avatars), NSPrivacyCollectedDataTypeAudioData (voice messages), NSPrivacyCollectedDataTypeOtherDataTypes (arbitrary file attachments), NSPrivacyCollectedDataTypePreciseLocation (location attachments), NSPrivacyCollectedDataTypeUserID (username / server user id), NSPrivacyCollectedDataTypeName (display_name), NSPrivacyCollectedDataTypeDeviceID (APNs + PushKit tokens). In the App Review Notes, add one paragraph explaining that the label describes the pre-configured default server the developer operates, and that a user who points the app at their own server shares nothing with the developer.

**F29 — attributesOfItemAtPath: is compiled into the app binary but NSPrivacyAccessedAPICategoryFileTimestamp is not declared**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/Core/MediaPrep.swift:467-469 — `static func fileSize(of url: URL) -> Int { (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) as? Int ?? 0 }`, called from MediaPrep.swift:146, :153, :244, :280 and AudioRecorder.swift:105. Confirmed in the built binary: `strings -a .../dd-rel/Build/Products/Release-nettrash-iphonesimulator/FamilyConnect.app/FamilyConnect | grep -x 'attributesOfItemAtPath:error:'` returns a hit; the share-extension binary returns nothing. ios/FamilyConnect/PrivacyInfo.xcprivacy declares only NSPrivacyAccessedAPICategoryUserDefaults / CA92.1.

> **Fix.** Add to ios/FamilyConnect/PrivacyInfo.xcprivacy a second NSPrivacyAccessedAPITypes entry: NSPrivacyAccessedAPIType = NSPrivacyAccessedAPICategoryFileTimestamp with NSPrivacyAccessedAPITypeReasons = [C617.1] ("access the timestamps, size, or other metadata of files inside the app container") — which is literally what MediaPrep.fileSize does on files in tmp/the app container. Optionally also rewrite fileSize to `(try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0`, but declare C617.1 either way rather than betting on which symbol the scanner keys off.

**F30 — appstore.md instructs "Data Not Linked to You", which is wrong for an app with server-side accounts**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md:101 — "Declare: User Content (messages) and User ID (username), collected, App Functionality only, NOT used for tracking, \"Data Not Linked to You\" is reasonable since no email/phone/real name is required." Contradicted by docs/protocol.md:63 (`User {"id": 7, "username": "anna", …}`) and the whole message/attachment model, which stores every message and attachment against that user id on the server.

> **Fix.** Rewrite ios/docs/appstore.md:101 to: every declared type is Linked to the User (accounts are persistent server identities), used for App Functionality only, never for tracking; and expand the type list to User Content (messages, photos or videos, audio data, other user content/files), Location (precise), Identifiers (user id, device id/push token), Contact Info (name = display name). Keep the note that a build with no default server could plausibly declare Data Not Collected, but flag that Release-nettrash is not that build.

**F31 — No privacy policy or support page exists for Family Connect; App Store Connect will not accept the submission without a Privacy Policy URL**  
*both · blocker*

> **Evidence.** `grep -ril 'familyconnect\|family connect' /Users/nettrash/Develop/nettrash.me/nettrash-me` returns nothing; frontend/assets/appstore/ and frontend/assets/play/ carry exchange, geo, md and scan only. ios/docs/appstore.md:99 still has this as an unchecked box: "- [ ] Fill `[SUPPORT_EMAIL]`; set Support URL and Privacy Policy URL (a page on nettrash.me explaining both modes…)".

> **Fix.** Publish /appstore/familyconnect/privacy.html and /appstore/familyconnect/support.html on nettrash.me following the pattern already used for md and scan. The privacy page must state, per data type: what is collected, that on the default server (fc.nettrash.me) the developer is the operator and can technically access it, that it is stored in plaintext PostgreSQL, the retention and account-deletion behaviour (POST /api/v1/me/delete), that a self-hosted server shares nothing with the developer, and that calls use STUN servers configured by the server operator (Google's public STUN by default) so the peer's IP is seen by that host. Then fill both URLs in App Store Connect.

**F32 — PrivacyInfo.xcprivacy's NSPrivacyCollectedDataTypes stays an empty array even after the label is corrected**  
*both · nice-to-have*

> **Evidence.** ios/FamilyConnect/PrivacyInfo.xcprivacy, whole file: NSPrivacyTracking=false, NSPrivacyTrackingDomains=[], NSPrivacyCollectedDataTypes=<array/>, one NSPrivacyAccessedAPITypes entry (UserDefaults/CA92.1). No other .xcprivacy exists in the repo (`find ios -name '*.xcprivacy'` returns exactly this one file).

> **Fix.** Populate NSPrivacyCollectedDataTypes in ios/FamilyConnect/PrivacyInfo.xcprivacy with the same set filed in App Store Connect (see finding app-privacy-label-declares-no-collection), each entry with NSPrivacyCollectedDataTypeLinked=true, NSPrivacyCollectedDataTypeTracking=false, NSPrivacyCollectedDataTypePurposes=[NSPrivacyCollectedDataTypePurposeAppFunctionality].

**F33 — "No third-party SDKs of any kind" is stated in both the App Store description and the review notes while WebRTC.framework ships inside the app**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md:31 ("What it does not have: ads, analytics, tracking, or third-party SDKs of any kind.") and :63 ("- No third-party SDKs, no analytics, no ads, no tracking."). Contradicted by ios/FamilyConnect.xcodeproj/project.pbxproj:1048-1062 (XCRemoteSwiftPackageReference "https://github.com/stasel/WebRTC.git", product WebRTC linked into the app target) and by the built bundle: .../FamilyConnect.app/Frameworks/WebRTC.framework exists, 12.1 MB device slice. The same two lines also claim "text messages only. Voice and video calls are planned" — both false; calls, photos, video, voice notes, files, polls and location sharing all ship.

> **Fix.** Replace appstore.md:31 with: "What it does not have: ads, analytics, or tracking of any kind. The only third-party code in the app is Google's open-source WebRTC library, which carries the voice and video calls and sends nothing anywhere on its own." Replace appstore.md:63 with: "No analytics, no ads, no tracking, no advertising or attribution SDKs. One third-party library is embedded: WebRTC (open source, github.com/stasel/WebRTC 151.0.0), used solely as the media engine for peer-to-peer voice and video calls." Separately, rewrite the whole Description and Beta App Description: the "text messages only" / "voice and video calls are planned" / "no photos or other media" lines are all stale.

**F34 — Calls default to Google's public STUN server, so the app contacts a host the family does not own on every call — undisclosed in the description, review notes and privacy story**  
*both · nice-to-have*

> **Evidence.** server/src/config.rs:987-989 — `fn default_stun_urls() -> Vec<String> { vec!["stun:stun.l.google.com:19302".to_string()] }`, applied at config.rs:119 and asserted at config.rs:1248. server/config.example.toml:190 `stun_urls = ["stun:stun.l.google.com:19302"]`. docs/protocol.md:1113-1127 and :1260 (`GET /calls/ice`). Client side: ios/FamilyConnect/FamilyConnectApp.swift:136 `calls.iceServers = { try await coordinator.api.iceServers().iceServers }` → CallManager.swift:609/:730 → WebRTCClient.swift:94-95 `configuration.iceServers = iceServers.map { RTCIceServer(urlStrings: $0.urls, …) }`. TURN is empty by default (config.rs:79). I could NOT verify what the live fc.nettrash.me actually serves — that requires querying the deployed config.

> **Fix.** First establish what fc.nettrash.me's [calls] section actually sets. Then either point stun_urls at a STUN service the developer runs (removing the third party entirely) or keep it and disclose: add to appstore.md's PRIVACY notes "Voice and video calls use STUN to discover the two devices' network addresses. The STUN server list is chosen by the server operator; the default configuration names Google's public STUN server, which sees the connecting IP addresses and nothing else — no message, media or account data ever passes through it. Self-hosters can point this at their own server or disable it." Mirror the sentence in the privacy policy page.

**F35 — Link previews (arbitrary web hosts) and map previews (Apple) are ON by default and are not mentioned in the store copy or review notes**  
*both · nice-to-have*

> **Evidence.** ios/FamilyConnect/Core/LinkPreviewLoader.swift:6-14 header — "this is the one place the app talks to a host the family does not own. Every device that displays a linked message contacts that link"; :81 builds the ephemeral URLSession; :88 gates on AppSettings.linkPreviewsEnabled. ios/FamilyConnect/Storage/AppSettings.swift:30-31 — the key is `v1.linkPreviewsDisabled`, "Stores the DISABLED flag, so a missing key reads as 'on'", i.e. default ON; same at :33-36 for `v1.mapPreviewsDisabled`. ios/FamilyConnect/Views/LocationAttachmentView.swift:13-19 — "THE MAP IS DRAWN BY THIS DEVICE … rendering tiles means asking Apple for them, with the coordinate a family member deliberately sent."

> **Fix.** Add to the appstore.md PRIVACY block and to the privacy policy page: "Two optional features contact hosts outside your server, both on by default and both switchable off in Settings. Link previews: when a message contains an https link, your device fetches that page's title and image directly from the site — no cookies, no credentials, no referrer, nothing stored. Map previews: a shared location is drawn with Apple Maps, which means your device asks Apple for tiles at that coordinate." Consider whether either should default off given the product's positioning.

**F36 — The server can forward member message text to Azure OpenAI; whether fc.nettrash.me has it enabled is unverified and undisclosed**  
*server · should-fix*

> **Evidence.** server/src/config.rs:130-164 — the `[ai]` section, "OFF unless configured … which is the right default for something that sends text to a third party"; `pub enabled: bool` (default false), `endpoint` ("Azure OpenAI resource endpoint"), `deployment`, `model`. server/src/handlers_ai.rs (whole file) implements the assistant reply flow; docs/protocol.md:392, :408-412 describe the assistant chat and @ai mentions in the family chat. Client side the feature is live: ios/FamilyConnect/Storage/AppSettings.swift:41-46 stores `v1.assistant.userId` / `v1.assistant.name` "as GET /families/mine last reported it … telling the composer whether to offer @ai at all". I could NOT determine whether the deployed fc.nettrash.me enables it.

> **Fix.** Determine the [ai] setting on fc.nettrash.me before submitting. If disabled, say so explicitly in the App Review Notes ("the optional AI assistant is not enabled on the default server; no message text leaves it"). If enabled, the privacy policy must name Azure OpenAI as a processor, state that only the member's own assistant-chat text and @ai-mentioning family messages are sent, and the App Store description must disclose the assistant as a feature — right now the description does not mention it at all.

### `guidelines-ugc` (9)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F39 | Guideline 1.2: none of the four required UGC safeguards exist (no filter, no report, no block, no published contact info) | both | blocker |
| F40 | No Privacy Policy page and no Support page exist for Family Connect — both are mandatory App Store Connect fields | both | blocker |
| F41 | PrivacyInfo.xcprivacy declares NSPrivacyCollectedDataTypes as empty while the shipping build sends messages, media and location to a developer-operated server | both | should-fix |
| F42 | Guideline 2.3.1: the store description says text-only, no calls and 'no third-party SDKs of any kind' — all three are false | both | should-fix |
| F43 | Guideline 2.1: review notes are an unfilled template that never mentions calls, media or location, and a P2P call cannot be demonstrated by one reviewer on one device | both | should-fix |
| F44 | Guideline 2.1: no TURN relay is configured by default, so a reviewer behind carrier-grade or symmetric NAT will see calls fail to connect | server | should-fix |
| F45 | Age rating must not be 4+ and must declare unrestricted user communication plus the in-app AI chatbot; Kids Category must not be selected | both | nice-to-have |
| F46 | Archiving the 'FamilyConnect' scheme instead of 'FamilyConnect-nettrash' ships a build with no server, guaranteeing a 2.1 rejection | both | nice-to-have |
| F47 | Guideline 5.1.1: username + password with no email means a forgotten password is unrecoverable and support has no way to verify a user | both | nice-to-have |

**F39 — Guideline 1.2: none of the four required UGC safeguards exist (no filter, no report, no block, no published contact info)**  
*both · blocker*

> **Evidence.** Full-repo greps find zero moderation surface. iOS/macOS: `grep -rniE "block|report|mute|abuse|objectionable|moderat" --include='*.swift' ios/` returns only unrelated hits; the only 'Mute' string in ios/FamilyConnect/Localizable.xcstrings is the in-call microphone toggle (ios/FamilyConnectTests/CallViewTests.swift:101). Server: every route is listed in server/src/app.rs:33-183 — there is no report, block, mute or moderation endpoint, and no DELETE for a message (docs/protocol.md:1237-1246 lists POST/PATCH/PUT for messages, votes and reactions only). The sole recourse is owner-only member removal, gated at ios/FamilyConnect/Views/FamilyManageView.swift:285 (`if session.isOwner, member.role != "owner"`). A non-owner member has no action at all: ios/FamilyConnect/MacViews/MacMessageRow.swift:302-308 shows the only 'Delete' is for a locally failed send. No EULA/Terms is surfaced anywhere — grepping the 447 keys of ios/FamilyConnect/Localizable.xcstrings for 'terms|eula|licen|polic' returns only 'Privacy' (the link/map-preview section header, SettingsView.swift:312). The closed-family argument does NOT hold on the shipping build: server/src/handlers_auth.rs:115-155 `register` has no invite gate, no email, no allowlist and no [registration] switch, so any App Store user can register on https://fc.nettrash.me (the URL baked into Release-nettrash), and ios/docs/appstore.md:52 tells the reviewer this in writing: 'Registration is also fully open'. Invite codes are one tap from a public ShareLink (SettingsView.swift:288-293) and join_policy 'open' admits instantly (server/src/handlers_family.rs:355).

> **Fix.** Minimum implementation that clears 1.2, all four parts: (1) FILTER — add a per-user 'hide' for a message plus a client-side profanity/keyword filter toggle, or at minimum an explicit content-filter setting; a DELETE /chats/{id}/messages/{mid} (author + family owner) also counts as removal of objectionable material. (2) REPORT — add a 'Report' action to the message context menu on both platforms (MessageBubbleView.swift and MacViews/MacMessageRow.swift) posting to a new POST /api/v1/reports endpoint, plus a stated 24-hour response commitment in the review notes. (3) BLOCK — a per-user block list (server-side, so a blocked member's messages and calls never reach the blocker) reachable from the member roster and from a message; owner-only removal is not a substitute because it is unavailable to the person being abused. (4) CONTACT — a 'Contact Support' row in Settings and MacSettingsView opening mailto: a real address, and the same address on the Support URL page. Also add an EULA/Terms acceptance on the registration screen (ios/FamilyConnect/Views/AuthView.swift) with a zero-tolerance-for-objectionable-content clause; Apple asks for this specifically on UGC apps. If any of this cannot ship, the alternative that removes the 1.2 exposure entirely is to strip the default server from the App Store build so the app is BYO-server only — but that then breaks Guideline 2.1 testability, so building the four controls is the realistic path.

**F40 — No Privacy Policy page and no Support page exist for Family Connect — both are mandatory App Store Connect fields**  
*both · blocker*

> **Evidence.** The website repo has store/legal pages for four other apps and nothing for this one: `ls /Users/nettrash/Develop/nettrash.me/nettrash-me/frontend/assets/appstore/` → exchange, geo, md, scan. `grep -ril 'familyconnect|family connect|family.connect' frontend/` in that repo returns nothing. In this repo, ios/docs/appstore.md:100 still carries the unticked checklist item 'Fill [SUPPORT_EMAIL]; set Support URL and Privacy Policy URL', and ios/docs/appstore.md:70 still has the literal placeholder '[SUPPORT_EMAIL]'. `grep -rn 'nettrash.me|@nettrash' --include='*.swift' ios/FamilyConnect/` returns no support contact in the app, and README.md contains no support/contact/privacy section.

> **Fix.** Add frontend/assets/appstore/familyconnect/{privacy,support}.html to the nettrash-me repo following the existing exchange/geo/md/scan pattern, plus the home.rs store card. The privacy page must cover both modes explicitly (data on the developer-operated fc.nettrash.me vs. a family's own server), and must name every category actually collected: messages, photos, videos, voice notes, files, shared locations, username, display name, birthday day/month, avatar, push tokens, and — if [ai] is enabled on fc.nettrash.me — that assistant prompts are sent to Azure OpenAI (server/src/ai.rs:1). The support page must carry a real monitored email address, and that same address must be surfaced in-app (Settings) to satisfy 1.2(d).

**F41 — PrivacyInfo.xcprivacy declares NSPrivacyCollectedDataTypes as empty while the shipping build sends messages, media and location to a developer-operated server**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/PrivacyInfo.xcprivacy declares `<key>NSPrivacyCollectedDataTypes</key><array/>` — i.e. nothing collected — and only one accessed-API reason (UserDefaults CA92.1). But Release-nettrash bakes FC_DEFAULT_SERVER_URL = https://fc.nettrash.me into ios/FamilyConnect/Info.plist, and the app uploads message bodies, photos/videos (server/src/handlers_attachment.rs), voice notes, files, coordinates (Core/LocationProvider.swift → protocol.md 'Locations'), username/display name/birthday and APNs tokens (server/src/handlers_device.rs) to it. The repo's own doc already says so: ios/docs/appstore.md:101 — 'because the store build defaults to the developer-operated server, "Data Not Collected" is NOT defensible any more.' The manifest was never updated to match.

> **Fix.** Populate NSPrivacyCollectedDataTypes in ios/FamilyConnect/PrivacyInfo.xcprivacy with the types actually sent: NSPrivacyCollectedDataTypeOtherUserContent (photos/videos/messages/audio), NSPrivacyCollectedDataTypeUserID (username), NSPrivacyCollectedDataTypeName (display name), NSPrivacyCollectedDataTypeCoarseLocation/PreciseLocation (shared locations), each with Linked=false, Tracking=false, Purposes=[AppFunctionality]. Add the required-reason entry for file timestamp APIs (C617.1) covering MediaPrep.swift:468. Then set the App Store Connect App Privacy answers to exactly the same set, and confirm the WebRTC 151.0.0 XCFramework ships its own privacy manifest or that its data use is folded into yours.

**F42 — Guideline 2.3.1: the store description says text-only, no calls and 'no third-party SDKs of any kind' — all three are false**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md:31 — 'What it does not have: ads, analytics, tracking, or third-party SDKs of any kind. And the honest limits of version 1: text messages only. Voice and video calls are planned.' The shipping app has photos/videos/albums, voice messages, file attachments, a Share Extension, location sharing, polls, reactions, a generative-AI assistant, and P2P voice AND video calls over WebRTC with CallKit. The single SPM dependency is a third-party binary XCFramework, https://github.com/stasel/WebRTC.git @ 151.0.0. The default STUN server is Google's (server/src/config.example.toml:190, `stun_urls = ["stun:stun.l.google.com:19302"]`), so a device's public address is revealed to a third party during every call. ios/docs/appstore.md:5-8 also still declares account deletion an open BLOCKER although it shipped (ios/FamilyConnect/Views/DeleteAccountView.swift).

> **Fix.** Rewrite ios/docs/appstore.md's Description, Keywords, Beta App Description and 'What to Test' for the actual v1.0 feature set: family + 1:1 chats, photos/videos/albums, voice messages, file attachments, share extension, location sharing, polls, reactions, boards, read receipts, and P2P voice/video calls. Delete the 'no third-party SDKs of any kind' claim (WebRTC is one) or restate it precisely as 'no analytics, ads or tracking SDKs'. Disclose the Google STUN default and the Azure OpenAI assistant if enabled on fc.nettrash.me. Delete the stale 5.1.1(v) blocker banner at lines 5-8 and the matching checklist item at line 96.

**F43 — Guideline 2.1: review notes are an unfilled template that never mentions calls, media or location, and a P2P call cannot be demonstrated by one reviewer on one device**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md:49-57 still contains literal placeholders [DEMO_USER], [DEMO_PASS], [DEMO_USER_2], [DEMO_PASS_2], [INVITE_CODE], and line 70 [SUPPORT_EMAIL]; the checklist at lines 97-99 confirms the demo accounts and reviewer family have not been created. The HOW TO REVIEW steps (lines 55-58) cover only sign-in, families, text chat and push — nothing about photos, albums, voice notes, files, polls, location, the Share Extension or calls. Calls are strictly two-party P2P: server/src/handlers_call.rs relays offer/answer/candidates only ('The server never carries a call's audio'), and CallKit incoming needs a VoIP push to a second real device.

> **Fix.** Before submission: create the two demo accounts and the seeded reviewer family on fc.nettrash.me and replace every placeholder; put the owner credentials in App Review Information. For calls, do all three: (a) supply BOTH accounts' credentials in the notes and state plainly that a second device or a second simulator/Mac signed in as the second account is required — the macOS build of the same target is the cheapest second endpoint and should be named as such; (b) attach a screen recording (App Review Information → Attachment) showing a full outgoing and incoming call including the CallKit incoming screen, which is the accepted fallback for two-party features; (c) confirm in the notes that the demo server stays online with calls enabled for the review period. Extend HOW TO REVIEW to walk photos/albums, voice notes, file attachments, polls, location sharing and the Share Extension.

**F44 — Guideline 2.1: no TURN relay is configured by default, so a reviewer behind carrier-grade or symmetric NAT will see calls fail to connect**  
*server · should-fix*

> **Evidence.** server/src/config.rs:79-82 — 'TURN servers... Empty means no relay: calls that cannot connect directly simply fail'; the default is `turn_urls: Vec::new()` (config.rs:120). server/config.example.toml:192-197 keeps every turn_urls/turn_secret line commented out. Only Google STUN is on by default (config.example.toml:190). On iOS the same-LAN direct path additionally needs the Local Network permission (INFOPLIST_KEY_NSLocalNetworkUsageDescription is present, project.pbxproj:547) — a reviewer who declines that prompt loses the LAN path too.

> **Fix.** Verify /etc/family-connect/config.toml on fc.nettrash.me actually sets [calls] turn_urls plus turn_secret against a running coturn, and confirm calls_enabled/video_calls_enabled are true (GET /api/v1/me reports both). If no relay is deployed, stand one up before submission or disable calls on the review server and remove calls from the listing — do not ship a call button that fails on a NAT the reviewer is behind.

**F45 — Age rating must not be 4+ and must declare unrestricted user communication plus the in-app AI chatbot; Kids Category must not be selected**  
*both · nice-to-have*

> **Evidence.** The app is unfiltered person-to-person communication with media (Views/AttachmentView.swift, AttachmentAlbum.swift, AudioPlayerView.swift, LocationAttachmentView.swift) and no moderation of any kind (see finding ugc-safeguards-entirely-absent). It additionally ships a generative-AI chatbot: server/src/ai.rs:1 'The assistant: Azure OpenAI chat completions, streamed', owner-configurable per family from ios/FamilyConnect/Views/FamilyAssistantSettings.swift, and mentionable as @ai inside the family chat (docs/protocol.md:1237). LSApplicationCategoryType is public.app-category.social-networking (project.pbxproj:545). The app's display name is 'Family' (project.pbxproj:544) and the description pitches 'grandparents to kids' (ios/docs/appstore.md:19), which invites Kids Category scrutiny.

> **Fix.** In App Store Connect, answer the user-generated-content and messaging questions truthfully and declare the in-app chatbot; expect and accept the resulting 13+ or higher rating. Do NOT select the Kids Category and do not describe the app as being for children in the description. Once report/block/filter controls exist per the 1.2 fix, declare them as the in-app parental/moderation controls the questionnaire asks about, which is what keeps the rating from being pushed to the top band.

**F46 — Archiving the 'FamilyConnect' scheme instead of 'FamilyConnect-nettrash' ships a build with no server, guaranteeing a 2.1 rejection**  
*both · nice-to-have*

> **Evidence.** Two shared schemes exist. FamilyConnect.xcodeproj/xcshareddata/xcschemes/FamilyConnect.xcscheme:79 archives with buildConfiguration = "Release"; FamilyConnect-nettrash.xcscheme:67,88 archives with "Release-nettrash". Only Release-nettrash sets FC_DEFAULT_SERVER_URL = https://fc.nettrash.me, which ios/FamilyConnect/Info.plist expands into FCDefaultServerURL. A Release archive therefore ships FCDefaultServerURL empty and lands on ios/FamilyConnect/Views/ServerSetupView.swift with nothing to connect to. Nothing in the project guards against picking the wrong one, and the plainly-named 'FamilyConnect' scheme is the wrong one.

> **Fix.** Archive with the FamilyConnect-nettrash scheme only, and verify before upload: `plutil -extract FCDefaultServerURL raw <archive>/Products/Applications/FamilyConnect.app/Info.plist` must print https://fc.nettrash.me. Consider renaming the schemes (e.g. 'FamilyConnect (BYO server)' vs 'FamilyConnect (App Store)') so the store scheme is the unambiguous one.

**F47 — Guideline 5.1.1: username + password with no email means a forgotten password is unrecoverable and support has no way to verify a user**  
*both · nice-to-have*

> **Evidence.** server/src/handlers_auth.rs:115-155 `register` takes only username, display_name and password — no email, no phone, no recovery factor. The only password route is POST /api/v1/me/password (server/src/app.rs:37), which requires an authenticated session. There is no reset/forgot endpoint anywhere in server/src/app.rs:33-183, and no 'Forgot password' affordance in ios/FamilyConnect/Views/AuthView.swift. Family owners can reset a member's password (ios/FamilyConnect/Views/FamilyManageView.swift:42), but an owner who forgets their own password on a family they are sole member of is permanently locked out with no path back.

> **Fix.** Either (a) add an optional, clearly-labelled recovery email at registration used solely for password reset, or (b) document the situation explicitly on the Support page and in the review notes: 'accounts intentionally carry no email; a family owner can reset any member's password from Settings → Family, and a locked-out sole owner must create a new account.' Option (b) is sufficient for review as long as the Support URL exists. Sign in with Apple is NOT required here — verified that no third-party or social login is offered anywhere in the app.

### `store-metadata` (18)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F51 | No Privacy Policy page exists — the mandatory Privacy Policy URL cannot be filled for either platform | both | blocker |
| F52 | No Support page exists — the mandatory Support URL cannot be filled | both | blocker |
| F53 | Zero iPhone and iPad screenshots exist anywhere in the repo; iPad screenshots are mandatory because TARGETED_DEVICE_FAMILY = "1,2" | ios | should-fix |
| F54 | Zero Mac screenshots exist and no Mac capture workflow is described | macos | nice-to-have |
| F55 | The entire Mac App Store listing is missing from appstore.md — no Mac description, no plan for the shared app record | macos | should-fix |
| F56 | The Description, Promotional Text and Keywords describe a text-only app with no third-party code — all three claims are now false | both | should-fix |
| F57 | The app ships an AI assistant backed by a third-party service (Azure OpenAI) and it is disclosed nowhere in the store metadata or privacy draft | both | should-fix |
| F58 | Five [PLACEHOLDER] tokens are unfilled, including the demo account the app cannot be reviewed without | both | should-fix |
| F59 | The drafted App Privacy label covers only messages and username — it is missing photos, video, audio, files, location, contacts-adjacent data, device tokens and third-party AI sharing | both | should-fix |
| F60 | appstore.md's top-of-file BLOCKER note and checklist item 1 claim account deletion is unimplemented; it is implemented | both | should-fix |
| F61 | Beta App Description and What to Test still tell testers the app has no media and no calls | both | should-fix |
| F62 | Store name "Family Connect" versus on-device display name "Family" — decide which is the App Store name, and check availability | both | nice-to-have |
| F63 | Subtitle, categories, copyright, content rights, age rating and export compliance are not recorded anywhere | both | nice-to-have |
| F64 | ITSAppUsesNonExemptEncryption is absent from both Info.plist files, so every upload will stop on the export-compliance question | both | should-fix |
| F65 | No report-content or block-user affordance exists, while the app is user-to-user messaging on an openly-registerable developer-operated server | both | should-fix |
| F66 | The app ships 10 localizations but no store-listing localization decision is recorded | both | nice-to-have |
| F67 | CHANGELOG's only entry is v0.1.0 describing a text-only app while MARKETING_VERSION is 1.0, and 78 commits of features are unrecorded | both | should-fix |
| F68 | Archiving from the default "FamilyConnect" scheme produces a build with no default server, directly contradicting the App Review notes | process | nice-to-have |

**F51 — No Privacy Policy page exists — the mandatory Privacy Policy URL cannot be filled for either platform**  
*both · blocker*

> **Evidence.** `ls /Users/nettrash/Develop/nettrash.me/nettrash-me/frontend/assets/appstore/` → `exchange geo md scan` only; no familyconnect dir. `grep -rn appstore frontend/src/` returns privacy/support links for exchange, scan, geo, md and none for Family Connect. appstore.md:100 still lists this as an unticked box: "- [ ] Fill `[SUPPORT_EMAIL]`; set Support URL and Privacy Policy URL (a page on nettrash.me explaining both modes...)".

> **Fix.** Create /Users/nettrash/Develop/nettrash.me/nettrash-me/frontend/assets/appstore/familyconnect/privacy.html modelled on frontend/assets/appstore/scan/privacy.html (same standalone-HTML shape: <title>, h1 Privacy Policy, Effective date line, TL;DR, What we collect, Data stored on your device, Permissions, Third-party services, Tracking, Children's privacy, International transfers, Your rights, Changes, Contact). No routing change is needed — frontend/index.html:38 has `<link data-trunk rel="copy-file"...>`-style `rel="copy-dir" href="assets/appstore"`, so the file is served at https://nettrash.me/appstore/familyconnect/privacy.html. Content it MUST cover, given this app: (1) two distinct data-controller modes — the developer-operated default server https://fc.nettrash.me that the store build ships pointed at, versus a family's own self-hosted server where the developer receives nothing; (2) that messages and attachments are stored in PLAINTEXT in the server's PostgreSQL and object store (README.md:159: "Messages are stored in plaintext in *your* PostgreSQL") — do not imply end-to-end encryption; (3) the categories actually carried now: message text, photos, video, voice recordings, arbitrary files, shared locations, polls, board notes, avatars, call metadata; (4) APNs device tokens; (5) that WebRTC voice/video media is peer-to-peer but signalling and any relay traverse the server; (6) that mentioning the assistant sends message text to a third-party AI service (Azure OpenAI — server/src/ai.rs, server/config.example.toml:238-260) when the operator has configured one; (7) the in-app account-deletion path (POST /api/v1/me/delete) and what it erases; (8) a contact address and an effective date.

**F52 — No Support page exists — the mandatory Support URL cannot be filled**  
*both · blocker*

> **Evidence.** Same directory listing as above: frontend/assets/appstore/ has scan/support.html and md/support.html but no familyconnect/support.html. appstore.md:70 and :97-100 leave `[SUPPORT_EMAIL]` unfilled.

> **Fix.** Create frontend/assets/appstore/familyconnect/support.html modelled on frontend/assets/appstore/scan/support.html (h1, Contact with a `mailto:nettrash@nettrash.me?subject=Family%20Connect%20support` dl, What the app does, FAQ, Reporting a bug, Feature requests). It will be served at https://nettrash.me/appstore/familyconnect/support.html. The FAQ must answer the two questions this app uniquely generates: how to point the app at your own server ("Change server" on the sign-in screen) and what the assistant is/whether it can be turned off.

**F53 — Zero iPhone and iPad screenshots exist anywhere in the repo; iPad screenshots are mandatory because TARGETED_DEVICE_FAMILY = "1,2"**  
*ios · should-fix*

> **Evidence.** `find . -path ./.git -prune -o \( -iname '*.png' -o -iname '*.jpg' \) -print` excluding Assets.xcassets returns only android/fastlane/metadata/android/en-US/images/{icon.png, featureGraphic.png, phoneScreenshots/01-chats.png, 02-family-chat.png, 03-private-chat.png}. `find ios -iname '*fastlane*' -o -iname '*screenshot*'` returns nothing. The three Android shots are 1344×2992 (sips), an Android aspect ratio that no iPhone or iPad screenshot slot accepts, and they show the Compose UI, not the SwiftUI one — nothing is reusable.

> **Fix.** Capture and upload, per localization you publish: iPhone 6.9" portrait (1290×2796 or 1320×2868) and iPad 13" portrait (2048×2732 or 2064×2752) — 1 minimum, 10 maximum per size. I am confident these are the current required slots and pixel sizes as of the 2025 App Store Connect simplification (other iPhone/iPad sizes are down-scaled from these), but re-check the live "Media Manager" requirements in App Store Connect at upload time rather than trusting this list. Shoot on iPhone 16 Pro and a 13" iPad Pro simulator (both available per the environment note). Content should cover what the listing will now claim: family chat, an album/photo viewer, a video call, a voice message, and Settings→Family owner tools.

**F54 — Zero Mac screenshots exist and no Mac capture workflow is described**  
*macos · nice-to-have*

> **Evidence.** Same find results — nothing under ios/ at all. appstore.md contains no occurrence of "Mac", "macOS", or "Mac App Store" (grep of the file). The app nevertheless builds a Mac binary: project.pbxproj sets SUPPORTED_PLATFORMS including macosx, MACOSX_DEPLOYMENT_TARGET = 14.0, INFOPLIST_FILE[sdk=macosx*] = Info-macOS.plist, and ios/FamilyConnect/MacViews/ holds 10 Mac-specific views.

> **Fix.** Capture the Mac app (MacChatView/MacConversationView/MacFamilyView) at one of the accepted Mac sizes — 1280×800, 1440×900, 2560×1600, or 2880×1800 — 1 minimum, 10 maximum. Verify the accepted list in App Store Connect at upload time. Mac screenshots must show the Mac window chrome, not an iPad-style layout.

**F55 — The entire Mac App Store listing is missing from appstore.md — no Mac description, no plan for the shared app record**  
*macos · should-fix*

> **Evidence.** appstore.md has no mention of macOS anywhere; its title is "# App Store Connect — Family Connect" and every section (Promotional Text, Description, Keywords, Notes for App Review, Beta App Description, What to Test, Pre-submission checklist) is written for iOS only. Meanwhile the project ships one target with one bundle id building both: PRODUCT_BUNDLE_IDENTIFIER = me.nettrash.FamilyConnect for every configuration, SUPPORTED_PLATFORMS = "iphoneos iphonesimulator macosx".

> **Fix.** Decide and write down: (a) Because both platforms are built from ONE target with ONE bundle id (me.nettrash.FamilyConnect), the correct setup is a SINGLE app record with the macOS platform added to it (App Store Connect → the app → Add Platform → macOS), which is also what gives universal purchase — the same identifier across platforms is precisely the precondition. Do NOT create a second app record with a second bundle id. (b) Even on one record, the macOS platform has its OWN version metadata: description, keywords, promotional text, screenshots, what's new, and its own review submission; plan on writing a Mac variant of the description that drops iPhone-only language (CallKit, PushKit wake-to-ring — Info-macOS.plist deliberately omits UIBackgroundModes and its comment says "there is no PushKit wake-to-ring on macOS") and mentions the Mac window UI. Confirm in App Store Connect which fields are record-level (App Privacy, age rating) versus per-platform rather than assuming. (c) Mac App Store prerequisites already satisfied and worth noting in the checklist: the app is sandboxed (FamilyConnect-macOS.entitlements has com.apple.security.app-sandbox) and has a Mac category (INFOPLIST_KEY_LSApplicationCategoryType = public.app-category.social-networking).

**F56 — The Description, Promotional Text and Keywords describe a text-only app with no third-party code — all three claims are now false**  
*both · should-fix*

> **Evidence.** appstore.md:31 — "What it does not have: ads, analytics, tracking, or third-party SDKs of any kind. And the honest limits of version 1: text messages only. Voice and video calls are planned." Contradicted by: ios/FamilyConnect/Views/{AttachmentAlbum,AttachmentViewer,AlbumStackView,AudioPlayerView,CameraPicker,LocationAttachmentView,PollComposerView,PollBubbleView,BoardView,CallView,CallVideoView,CallRecordView,CallRequestLinkSheet}.swift; the FamilyConnectShareExtension target; and the SPM dependency https://github.com/stasel/WebRTC.git @151.0.0 (a third-party binary XCFramework), which alone falsifies "no third-party SDKs of any kind". appstore.md:24-29's feature bullet list omits media, calls, polls, boards, location, edits, reactions and the Share Extension entirely. appstore.md:37 Keywords — "chat,messenger,private,self-hosted,home,server,secure,messages,household,text,group,relatives" — spends a slot on "text" and none on photo/video/call/voice. appstore.md:12 Promotional Text likewise sells only "Private chat for just your family."

> **Fix.** Rewrite the Description around the shipped feature set (family chat + 1:1 chats, photos/videos/albums with a viewer, voice messages, file attachments, share-sheet sending, location sharing, polls, boards, message edits, reactions, read receipts, typing indicators, offline history, push, and peer-to-peer voice AND video calls with CallKit on iPhone). Delete "text messages only", "Voice and video calls are planned" and "third-party SDKs of any kind" (replace with a truthful line: no ads, no analytics, no trackers; the only third-party component is the WebRTC library that carries calls). Rewrite Keywords to spend the 100 chars on the real feature surface (photos, video, calls, voice, self-hosted...) and drop "text". Rewrite the 170-char Promotional Text to mention calls and photos.

**F57 — The app ships an AI assistant backed by a third-party service (Azure OpenAI) and it is disclosed nowhere in the store metadata or privacy draft**  
*both · should-fix*

> **Evidence.** docs/protocol.md:460-558 ("### The assistant", "Mentioning the assistant in the family chat", per-member private `ai` chats, the `assistant: {user_id, display_name, mention: "@ai"}` capability field); ios/FamilyConnect/Views/FamilyAssistantSettings.swift (owner-only assistant language + context settings, mirrored on Mac in MacFamilyView); server/src/ai.rs and server/src/handlers_ai.rs; server/config.example.toml:238-260 configures an Azure OpenAI endpoint. appstore.md never contains the words "assistant", "AI" or "@ai", and appstore.md:63 asserts "No third-party SDKs, no analytics, no ads, no tracking."

> **Fix.** Decide first whether the assistant is ENABLED on fc.nettrash.me for the review build (protocol.md:532 — the `assistant` field is absent when the server is not configured for one, and clients then hide @ai). If enabled: describe it in the Description, disclose it in App Review Notes with a step showing how to reach it, name the processor (Microsoft Azure OpenAI) in the privacy policy, and reflect the sharing in the App Privacy label. If disabled for launch: say so explicitly in the review notes so a reviewer who reads the open-source server does not think you concealed it, and keep the description silent about it.

**F58 — Five [PLACEHOLDER] tokens are unfilled, including the demo account the app cannot be reviewed without**  
*both · should-fix*

> **Evidence.** appstore.md:49 `[DEMO_USER]`, `[DEMO_PASS]`; :50 `[DEMO_USER_2]`, `[DEMO_PASS_2]`; :57 `[INVITE_CODE]`; :70 `[SUPPORT_EMAIL]`. Checklist items appstore.md:97-100 all still unticked.

> **Fix.** [DEMO_USER]/[DEMO_PASS] — create the owner account on fc.nettrash.me and enter the same pair into App Review Information → Sign-In Required → User Name / Password. [DEMO_USER_2]/[DEMO_PASS_2] — second account for the two-device real-time/read-receipt demo (goes in the notes text only, not the ASC fields, which take one pair). [INVITE_CODE] — the reviewer family's invite code, with join policy set to instant. [SUPPORT_EMAIL] — nettrash@nettrash.me, matching the Support URL page. Also re-seed the reviewer family with the features the new description will claim (a photo album, a voice message, a poll, a shared location) so the reviewer sees them without having to construct them.

**F59 — The drafted App Privacy label covers only messages and username — it is missing photos, video, audio, files, location, contacts-adjacent data, device tokens and third-party AI sharing**  
*both · should-fix*

> **Evidence.** appstore.md:101 — "Declare: User Content (messages) and User ID (username), collected, App Functionality only, NOT used for tracking, 'Data Not Linked to You' is reasonable since no email/phone/real name is required." The app now also carries: photos/video (Views/AttachmentAlbum.swift, CameraPicker.swift, INFOPLIST_KEY_NSPhotoLibraryAddUsageDescription), voice recordings (AudioPlayerView.swift, INFOPLIST_KEY_NSMicrophoneUsageDescription), arbitrary files (Share Extension + MacFilePicker.swift), precise location (Views/LocationAttachmentView.swift, INFOPLIST_KEY_NSLocationWhenInUseUsageDescription, macOS entitlement com.apple.security.personal-information.location), APNs device tokens (server/src/push.rs, handlers_device.rs), call metadata (server/src/handlers_call.rs, Views/CallRecordView.swift), and AI processing (server/src/ai.rs).

> **Fix.** Re-answer the questionnaire for the store build (which defaults to the developer-operated fc.nettrash.me, so "Data Not Collected" is not available): User Content → Photos or Videos, Audio Data, Customer Support/Other User Content (messages, files, polls, boards); Identifiers → User ID (username); Location → Precise Location (user-initiated shares); Contact Info → none (no email/phone is ever requested — keep that true); Usage Data → none; Diagnostics → none. Purpose: App Functionality for all. Tracking: No, for all. "Linked to You" — re-examine the draft's "Not Linked" claim: everything is stored against a server-side account, and Apple's definition of linked is linkage to a user identity, not to a real name; a username-based account is still an identity, so Linked to You is the defensible answer for User Content and User ID. Add the third-party AI processor to the disclosure if the assistant is enabled.

**F60 — appstore.md's top-of-file BLOCKER note and checklist item 1 claim account deletion is unimplemented; it is implemented**  
*both · should-fix*

> **Evidence.** appstore.md:5-8 — "**BLOCKER before submitting** (guideline 5.1.1(v)): ... v1 does not have one yet — it needs a `DELETE /me` server endpoint plus a Settings → Account → Delete Account screen ... implement it first or expect a rejection." and appstore.md:96 — "- [ ] Implement account deletion (server `DELETE /me` + Settings → Account → Delete Account) — see blocker above". Contradicted by server/src/handlers_auth.rs::delete_account behind POST /api/v1/me/delete, ios/FamilyConnect/Views/DeleteAccountView.swift, ios/FamilyConnect/MacViews/MacSettingsView.swift, and ios/FamilyConnectTests/AccountDeletionTests.swift. Note the route is POST /me/delete, not DELETE /me as the note asserts.

> **Fix.** Delete the blockquote at appstore.md:5-8 and tick/remove checklist item :96. Keep appstore.md:60 ("ACCOUNT DELETION (guideline 5.1.1(v)): Settings > Account > Delete Account...") and verify its wording matches what the shipped screen actually does on the Mac too.

**F61 — Beta App Description and What to Test still tell testers the app has no media and no calls**  
*both · should-fix*

> **Evidence.** appstore.md:80 — "What is not here yet, on purpose: photos and other media, and voice/video calls. Text only for now." appstore.md:78 lists only messaging/push/offline/receipts/typing. appstore.md:88-92 ("## What to Test (first TestFlight build)") walks a tester through register → invite → text messages → push, and never mentions photos, albums, voice messages, files, share sheet, location, polls, boards, reactions, edits or a single call. appstore.md:66 in the review notes likewise: "Text messages only in this version."

> **Fix.** Rewrite the Beta App Description and What to Test around the real surfaces, with explicit call-testing steps (call between two devices on the same Wi-Fi and on cellular — the local-network permission at INFOPLIST_KEY_NSLocalNetworkUsageDescription is what makes same-LAN calls work, and its denial is silent), media send/receive, the share sheet ("Share with FamilyConnect"), and the Mac app if it goes to TestFlight too.

**F62 — Store name "Family Connect" versus on-device display name "Family" — decide which is the App Store name, and check availability**  
*both · nice-to-have*

> **Evidence.** project.pbxproj sets INFOPLIST_KEY_CFBundleDisplayName = Family in every app-target configuration (lines 544, 597, 806). The ten AppIntentVocabulary.plist files are built around the spoken name "Family". appstore.md is titled and written throughout as "Family Connect", as is android/fastlane/metadata/android/en-US/title.txt ("Family Connect").

> **Fix.** Reserve the App Store name as "Family Connect" (matching the Android title and this doc), and keep CFBundleDisplayName = "Family" only if you accept the mismatch — it is defensible because "Family" is the leading word of the store name, which is the usual test. Verify the name is free in App Store Connect before anything else; if "Family Connect" is taken, decide the fallback now (e.g. "nettrash Family Connect", matching the sibling apps' "nettrash md" / "nettrash Scan" convention visible in nettrash-me/frontend/src/components/home.rs).

**F63 — Subtitle, categories, copyright, content rights, age rating and export compliance are not recorded anywhere**  
*both · nice-to-have*

> **Evidence.** appstore.md contains a Promotional Text, Description, Keywords, review notes and TestFlight text — and nothing else. There is no subtitle, no category, no copyright line, no age-rating answers and no export-compliance answer in the file. The only category signal in the repo is INFOPLIST_KEY_LSApplicationCategoryType = public.app-category.social-networking (project.pbxproj:545/598/807).

> **Fix.** Record decisions in appstore.md for: Subtitle (30 chars max, e.g. "Private chat for one family"); Primary category — Social Networking, matching LSApplicationCategoryType; Secondary category — decide (Utilities or none); Copyright — "2026 nettrash" per the repo's identity convention; Content Rights — the app contains no third-party content, answer accordingly; Age Rating questionnaire — must be answered honestly for an app with user-to-user messaging and (if enabled) an AI chatbot, see the separate UGC finding; Export compliance — see the ITSAppUsesNonExemptEncryption finding; Sign-in required = Yes with the demo credentials; "Is your app designed for kids / Kids Category" = No (do not opt in — a Kids Category app with open user-to-user chat is disallowed).

**F64 — ITSAppUsesNonExemptEncryption is absent from both Info.plist files, so every upload will stop on the export-compliance question**  
*both · should-fix*

> **Evidence.** `grep -rn ITSAppUsesNonExemptEncryption` over the whole repo returns nothing; neither ios/FamilyConnect/Info.plist nor ios/FamilyConnect/Info-macOS.plist contains it. The app does use encryption: HTTPS/TLS to the server, and WebRTC's DTLS-SRTP for call media (SPM dependency stasel/WebRTC 151.0.0). `grep -rln "CryptoKit\|CommonCrypto" ios/FamilyConnect/` returns nothing, so there is no custom or non-standard cryptography.

> **Fix.** Decide the answer once and record it: the app uses only standard TLS/HTTPS and the platform/WebRTC DTLS-SRTP, which is the exempt category. Then set ITSAppUsesNonExemptEncryption = NO in BOTH ios/FamilyConnect/Info.plist and ios/FamilyConnect/Info-macOS.plist (the macOS file's own comment already says "Keep the two files in step"). Note this is a code change, so it belongs to whoever owns the build, not to the metadata pass — but the ANSWER is a metadata decision and must be written into appstore.md either way.

**F65 — No report-content or block-user affordance exists, while the app is user-to-user messaging on an openly-registerable developer-operated server**  
*both · should-fix*

> **Evidence.** `grep -o '"[^"]*[Rr]eport[^"]*"' ios/FamilyConnect/Localizable.xcstrings` and the same for "block" both return zero string keys — there is no report or block UI anywhere in the 650 KB catalogue. ios/FamilyConnect/Views/ has no such screen. appstore.md:102 itself flags the exposure: "Decide whether open registration on fc.nettrash.me is acceptable long-term — any App Store user can register and create their own family on your box".

> **Fix.** Either (a) close registration on fc.nettrash.me before submission — appstore.md:102 already floats "A [registration] config switch on the server" — and say in the App Review Notes that the demo accounts are pre-provisioned and registration is closed, which narrows the 1.2 surface to a private family; or (b) keep open registration and add block/report affordances. Whichever is chosen, answer the age-rating questionnaire's messaging/UGC questions truthfully and expect a rating above 4+, and put the reasoning in appstore.md so it is not re-litigated at submission time.

**F66 — The app ships 10 localizations but no store-listing localization decision is recorded**  
*both · nice-to-have*

> **Evidence.** ios/FamilyConnect has .lproj directories for Base, en, ru, de, es, fr, ja, sr, sr-Latn, zh-Hans plus 10 AppIntentVocabulary.plist files; ios/FamilyConnect/Localizable.xcstrings is ~650 KB. appstore.md contains exactly one language's copy and no note about the others. android/fastlane/metadata has only en-US, so there is no reusable translated store copy either.

> **Fix.** Record the decision explicitly in appstore.md. Recommended for a first release: English only, matching what the sibling apps did (Geo shipped English-only, and md's Play listing is en-GB only). If any locale is added, note that each one needs its own Name, Subtitle, Description, Keywords, Promotional Text AND its own screenshot set — and that Russian is the highest-value addition given the app's Cyrillic-first user base implied by the ru and sr localizations.

**F67 — CHANGELOG's only entry is v0.1.0 describing a text-only app while MARKETING_VERSION is 1.0, and 78 commits of features are unrecorded**  
*both · should-fix*

> **Evidence.** CHANGELOG contains a single entry, "2026-08-19 v0.1.0 — Initial version. Self-hosted family chat: Rust server ... SwiftUI iOS client (iOS 17+) and Jetpack Compose Android client (Android 8+), both with offline history caches, optimistic sending with retry, reconnect resync, unread counts, read markers and typing indicators" — no mention of media, calls, polls, boards, location, the assistant or the Share Extension. project.pbxproj has MARKETING_VERSION = 1.0 in every configuration. `git log --oneline --since=2026-08-19 | wc -l` → 78. CURRENT_PROJECT_VERSION now reads 103 in project.pbxproj (context stated 101 — the schemes' `agvtool bump` Build PostAction raised it during earlier builds; expected, not a defect).

> **Fix.** Add a CHANGELOG entry for v1.0 dated at release, covering the 78 commits of shipped work: media and albums with a viewer, voice messages, file attachments, the Share Extension, location sharing, polls, boards, message edits, reactions, WebRTC voice and video calls with CallKit, the macOS app, the assistant, and account deletion. Leave "What's New in This Version" empty/absent for the 1.0 submission on both platforms — for a first release the Description carries the story. If the field is offered anyway, a single line ("First release.") is correct; do not paste a changelog into it.

**F68 — Archiving from the default "FamilyConnect" scheme produces a build with no default server, directly contradicting the App Review notes**  
*process · nice-to-have*

> **Evidence.** FamilyConnect.xcodeproj/xcshareddata/xcschemes/FamilyConnect.xcscheme sets buildConfiguration = "Release" for its Archive action (line 99); FamilyConnect-nettrash.xcscheme sets "Release-nettrash" (line 108). Only Release-nettrash defines FC_DEFAULT_SERVER_URL = https://fc.nettrash.me — Debug and Release leave it empty, and Info.plist's FCDefaultServerURL comment confirms: "empty in Debug/Release (first run asks for a server), https://fc.nettrash.me in Release-nettrash (the App Store build lands straight on Register/Login)". appstore.md:43 and :55 promise the reviewer the opposite: "launch it and the sign-in screen appears directly" / "Launch the app — it is already pointed at the default server".

> **Fix.** Add an explicit line to the Pre-submission checklist in ios/docs/appstore.md: archive ONLY from the FamilyConnect-nettrash scheme (Release-nettrash), for both the iOS and the macOS destination, and verify FCDefaultServerURL in the exported .app's Info.plist reads https://fc.nettrash.me before uploading. Also add a post-upload sanity step: install the TestFlight build on a clean device and confirm the sign-in screen appears with no server prompt.

### `server-readiness` (12)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F70 | App Review cannot exercise chat, calls, or any core feature — a fresh account on fc.nettrash.me is alone and can reach nobody | server | should-fix |
| F71 | No rate limiting anywhere: two unauthenticated argon2id endpoints on a single box, plus unlimited account creation | server | should-fix |
| F72 | coturn is deployed and healthy, but nothing proves the server actually hands its URLs out — a STUN-only /calls/ice would still look fine | server | should-fix |
| F73 | coturn serves the certbot certificate on 5349 but nothing reloads coturn when certbot renews it | server | nice-to-have |
| F74 | APNs environment on the live box is unverifiable, and getting it wrong delivers exactly nothing to a TestFlight or App Store build | server | should-fix |
| F75 | No per-user or per-family storage quota and no free-space check: any account can fill the disk for every family | server | should-fix |
| F76 | No backup procedure exists anywhere in the repo — a disk loss destroys every family's history and media irrecoverably | server | should-fix |
| F77 | Default retention deletes every message and its photos/videos after 100 days — unverified on production and undisclosed to users | server | should-fix |
| F78 | Open registration on a personally-owned box with no report, block, or moderation mechanism anywhere in the protocol | server | should-fix |
| F79 | One unmonitored box is the whole service, and healthz exposes nothing that identifies the running build | server | should-fix |
| F80 | 190 unit tests pass and lint is clean, but the 239 tests that actually exercise the server have never run automatically | process | nice-to-have |
| F81 | If [ai] is enabled on the production box, member text leaves to Azure OpenAI — unverifiable from here and undisclosed | server | should-fix |

**F70 — App Review cannot exercise chat, calls, or any core feature — a fresh account on fc.nettrash.me is alone and can reach nobody**  
*server · should-fix*

> **Evidence.** Registration creates a user with no family: `INSERT INTO users (username, display_name, password_hash)` with no family_id — /Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_auth.rs:125-135. A direct chat then requires a family AND a same-family peer: `handlers_chat::direct_chat` returns `not_in_family` when `auth.family_id` is None (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_chat.rs:1297-1303) and `not_same_family` when the peer's family_id differs (same file, 1311-1316). A call requires that direct chat: `handlers_call::offer` refuses with `invalid_call` "voice calls are only allowed in a direct chat" (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_call.rs, kind != "direct" branch), and joining an existing family needs an 8-character invite code (`tokens::gen_invite_code`, 30^8 space). The shipped build lands the reviewer on this exact server: `FC_DEFAULT_SERVER_URL = "https://fc.nettrash.me"` in the Release-nettrash config (/Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect.xcodeproj/project.pbxproj:769), adopted automatically by AppSession.bootstrap (/Users/nettrash/Develop/nettrash.me/family.connect/ios/FamilyConnect/Core/AppSession.swift:313-320).

> **Fix.** Before submitting, provision on fc.nettrash.me: (a) a demo family with a seeded history (messages, a photo album, a voice note, a poll) and its invite code, (b) TWO demo accounts in that family so the reviewer can sign in on two devices/simulators and place a real call between them, and (c) a second account that stays signed in on a real device so the reviewer can ring it. Put the username/password for both, plus the invite code, in App Store Connect's App Review Information notes, and add the same to ios/docs/appstore.md. Keep those accounts exempt from the retention sweep (see the retention finding) so the seeded history does not vanish.

**F71 — No rate limiting anywhere: two unauthenticated argon2id endpoints on a single box, plus unlimited account creation**  
*server · should-fix*

> **Evidence.** The router in /Users/nettrash/Develop/nettrash.me/family.connect/server/src/app.rs applies exactly one layer — `DefaultBodyLimit` — and no rate-limit/tower-governor layer; Cargo.toml lists no limiter crate. The shipped nginx site /Users/nettrash/Develop/nettrash.me/family.connect/server/nginx/family-connect.conf has no `limit_req` or `limit_conn` zone anywhere. `POST /auth/register` runs a full argon2id hash (`auth::hash_password`, handlers_auth.rs:123), and `POST /auth/login` runs an argon2id verification even for a username that does not exist — deliberately, to close a timing oracle: "it would skip the argon2 work that keeps the paths indistinguishable by timing" (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/auth.rs:143-155). Config validation (config.rs:728-866) has no registration switch, no per-IP setting, and no `registration_open` key exists in LimitsConfig (config.rs:388-489).

> **Fix.** Add `limit_req_zone $binary_remote_addr zone=fcauth:10m rate=10r/m;` plus `limit_req zone=fcauth burst=5 nodelay;` on `location = /api/v1/auth/login` and `= /api/v1/auth/register` in server/nginx/family-connect.conf, and a looser `limit_req` (e.g. 20r/s) plus `limit_conn` on the whole `/api/v1/` block. Ship the hardened conf in the repo so the deployment is reproducible. Longer term, add a `[limits] registration_open` switch to config.rs so nettrash's own box can be closed to new sign-ups once the intended families are on it.

**F72 — coturn is deployed and healthy, but nothing proves the server actually hands its URLs out — a STUN-only /calls/ice would still look fine**  
*server · should-fix*

> **Evidence.** Live probe: a STUN Binding Request to 92.205.163.161 (fc.nettrash.me) answers on BOTH udp/3478 and tcp/3478 with `type=0x0101` and `SOFTWARE = "Coturn-4.6.1 'Gorst'"`; a TURN Allocate without credentials answers `0x0113` error `401 Unauthorized` carrying `NONCE` and `REALM = "fc.nettrash.me"`; tcp/5349 completes a TLS handshake presenting the same `CN=fc.nettrash.me` Let's Encrypt certificate. So a correctly-realmed, credentialed coturn IS running. But the server only emits a TURN entry when config says so: `if !calls.turn_urls.is_empty()` gates the entire TURN block in `handlers_call::ice_servers` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_call.rs:115), `turn_urls` is commented out in the shipped example (/Users/nettrash/Develop/nettrash.me/family.connect/server/config.example.toml:197), and `GET /api/v1/calls/ice` on the live host requires auth (verified: `401 {"error":{"code":"unauthorized"...}}`), so /etc/family-connect/config.toml cannot be read from here. The README documents no TURN/coturn step at all (grep for coturn/turn_urls/3478/stun in README.md: zero hits).

> **Fix.** Log in once with a real account and confirm `GET https://fc.nettrash.me/api/v1/calls/ice` returns an entry whose `urls` contain both `turn:...:3478` and `turns:...:5349` with a non-null `username`/`credential`. Then place one real relayed call end-to-end (force it by disabling host/srflx candidates or calling from a mobile carrier network). Also add the coturn install + `static-auth-secret` + `realm` + certificate steps to README.md so the production deployment is reproducible from the repo.

**F73 — coturn serves the certbot certificate on 5349 but nothing reloads coturn when certbot renews it**  
*server · nice-to-have*

> **Evidence.** `openssl s_client -connect 92.205.163.161:5349` presents `subject=CN=fc.nettrash.me`, `issuer=C=US, O=Let's Encrypt, CN=YE1`, `notAfter=Nov 17 19:46:58 2026 GMT` — the identical certificate served on 443, so coturn is pointed at the certbot-managed `/etc/letsencrypt/live/fc.nettrash.me/` files. The repo's only TLS instruction is `sudo certbot --nginx -d chat.yourdomain.tld` (/Users/nettrash/Develop/nettrash.me/family.connect/README.md:135 and server/nginx/family-connect.conf header), which installs a renewal hook that reloads nginx and nothing else; there is no coturn deploy hook, no coturn unit, and no coturn config anywhere in the repo.

> **Fix.** Add `/etc/letsencrypt/renewal-hooks/deploy/coturn.sh` containing `systemctl reload coturn || systemctl restart coturn`, make it executable, and verify with `certbot renew --dry-run`. Commit the hook (and the coturn turnserver.conf skeleton) into server/ alongside the nginx and systemd artifacts so the deployment is reproducible.

**F74 — APNs environment on the live box is unverifiable, and getting it wrong delivers exactly nothing to a TestFlight or App Store build**  
*server · should-fix*

> **Evidence.** The transport itself is correct and complete: ES256 provider JWT with `kid`/`iss`/`iat` and a mandatory 45-minute cache (`APNS_JWT_REFRESH`, /Users/nettrash/Develop/nettrash.me/family.connect/server/src/push.rs:196-262), `apns-topic: <bundle_id>` + `apns-push-type: alert` + `apns-priority: 10` for messages (push.rs:309-322), and a properly separate VoIP channel: `apns-topic: {bundle_id}.voip`, `apns-push-type: voip`, `apns-priority: 10`, `apns-expiration: now + ring_timeout_secs` (push.rs:385-405) — no separate certificate is needed because token-based .p8 auth covers the `.voip` topic. The endpoint is derived from config: `"sandbox" => https://api.sandbox.push.apple.com`, else `https://api.push.apple.com` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/config.rs:543-551), and the default is right (`default_apns_environment()` returns `"production"`, config.rs:1002-1004). But the deployed /etc/family-connect/config.toml is not in the repo and cannot be read remotely, and config.example.toml ships the whole `[push.apns]` block commented out (config.example.toml, `# environment = "production"` and `# bundle_id = "me.nettrash.FamilyConnect"`).

> **Fix.** On the box: `grep -A8 '\[push.apns\]' /etc/family-connect/config.toml` and confirm `environment = "production"`, `bundle_id = "me.nettrash.FamilyConnect"` (exact capitalization — a mismatch is `TopicDisallowed` on every push), and that `key_file` points at a readable AuthKey_*.p8 owned root:family-connect mode 0640. Then send yourself one real alert push and one real VoIP push to a TestFlight build before submitting, and `journalctl -u family-connect | grep -i apns` to confirm no rejections.

**F75 — No per-user or per-family storage quota and no free-space check: any account can fill the disk for every family**  
*server · should-fix*

> **Evidence.** `LimitsConfig` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/config.rs:388-489) has ceilings per attachment (`max_attachment_bytes`, 104857600 = 100 MB, config.example.toml:60) and per message (`max_attachments_per_message`, 10) but no quota field of any kind — no per-user bytes, no per-family bytes, no total-disk cap. `Storage::write_stream` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/storage.rs:76-140) counts bytes against `max_bytes` and never consults free space. Config validation (config.rs:728-866) has no disk check. Combined with open registration (handlers_auth.rs:115-157, no invite or switch), one authenticated account can upload 1 GB per message, repeatedly. The only bound is the retention sweep at 100 days (config.example.toml:87).

> **Fix.** Add a `[limits] max_storage_bytes_per_user` (and/or per family) enforced in `handlers_attachment::upload_attachment` by summing `attachments.bytes` for the uploader before accepting the write, returning a new `storage_quota_exceeded` protocol error. As an immediate stopgap before submission, put the attachments directory on its own filesystem or LVM volume so a fill cannot take PostgreSQL down with it, and add a disk-usage alert at 80%.

**F76 — No backup procedure exists anywhere in the repo — a disk loss destroys every family's history and media irrecoverably**  
*server · should-fix*

> **Evidence.** The only mention of backups in the whole repo is a warning, not a procedure: "NOTE FOR BACKUPS: because of that, a database dump is no longer a complete backup — this directory has to be backed up alongside it" (/Users/nettrash/Develop/nettrash.me/family.connect/server/config.example.toml:147-148) and the same sentence in docs/protocol.md:912-913. `grep -ni 'backup|pg_dump|restore' README.md` returns zero hits; server/scripts/ contains only `seed-album-uitest.sh`, `seed-scroll-uitest.sh` and `uitest-server.toml`; the systemd unit (/Users/nettrash/Develop/nettrash.me/family.connect/server/systemd/family-connect.service) has no backup timer and no companion .timer unit exists.

> **Fix.** Add a documented backup to README.md and ship a systemd timer in server/systemd/: nightly `pg_dump -Fc family_connect` plus an rsync/restic of /var/lib/family-connect/attachments to off-box storage, with a documented restore drill (restore the dump, restore the directory, start the service, confirm one attachment opens). Test the restore once before submitting.

**F77 — Default retention deletes every message and its photos/videos after 100 days — unverified on production and undisclosed to users**  
*server · should-fix*

> **Evidence.** `retention_days = 100` is the shipped default (/Users/nettrash/Develop/nettrash.me/family.connect/server/config.example.toml:87 and `default_retention_days` in config.rs), and the sweep runs at boot and hourly thereafter with no opt-out per user: `handlers_chat::sweep_expired_messages` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_chat.rs:193) called from the sweeper loop in /Users/nettrash/Develop/nettrash.me/family.connect/server/src/main.rs:90-101. protocol.md:897 confirms "The server deletes messages older than `limits.retention_days` (**100 days** by default), together with" their attachments. The live config cannot be read remotely, so whether nettrash set 0 (keep everything) is unknown.

> **Fix.** Decide and confirm the production value (`grep retention_days /etc/family-connect/config.toml`). Whichever it is, state it plainly in the App Store description and the (still-missing) privacy policy page — e.g. "messages and media are kept on the server for N days" — and surface it in the app's settings so a family knows. If the intent is to keep everything, set `retention_days = 0` explicitly rather than relying on nobody having noticed the default.

**F78 — Open registration on a personally-owned box with no report, block, or moderation mechanism anywhere in the protocol**  
*server · should-fix*

> **Evidence.** The complete route table (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/app.rs:19-186) contains no report, block, mute, or flag endpoint; `grep -rni 'block user|report abuse|report content|objectionable|moderat'` across docs/protocol.md, README.md and ios/docs/appstore.md returns zero hits. Registration is unconditional and unswitchable (handlers_auth.rs:115-157). The mitigating factor is real and worth stating: a stranger cannot reach anyone — `handlers_chat::direct_chat` refuses `not_same_family` (handlers_chat.rs:1311-1316), and joining a family needs an 8-char invite code from a 30^8 space (`tokens::gen_invite_code`) with the owner optionally gating it (`join_policy = "approval"`, handlers_family.rs:355). The owner can remove members (`DELETE /families/members/{user_id}`) but there is no way for a member to block or report anyone.

> **Fix.** For review, lead with the containment argument in the App Review notes — content is only ever visible inside an invite-gated family, an owner approves or rejects every join and can remove any member — and add the minimum 1.2 surface: a per-member "remove from family" already exists, so add a "Report" action in the app that emails the operator, and publish operator contact details on the (missing) support page. Operationally, add a `[limits] registration_open = false` switch so the box can be closed to strangers once the intended families are on it.

**F79 — One unmonitored box is the whole service, and healthz exposes nothing that identifies the running build**  
*server · should-fix*

> **Evidence.** `GET /api/v1/healthz` returns exactly `{"status":"ok"}` — no version, no build, no migration level: `Ok((StatusCode::OK, Json(json!({"status": "ok"}))).into_response())` (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_device.rs:160-169). The only resilience is `Restart=on-failure` / `RestartSec=5s` in /Users/nettrash/Develop/nettrash.me/family.connect/server/systemd/family-connect.service; there is no health-check timer, no alerting, no second host, and `.github/` has no workflows. Response headers from the live host disclose `Server: nginx/1.24.0 (Ubuntu)` and carry no `Strict-Transport-Security`. Every App Store user and the reviewer land here by default (project.pbxproj:769).

> **Fix.** Add external uptime monitoring on `https://fc.nettrash.me/api/v1/healthz` with alerting to a channel nettrash actually reads, before submitting. Extend healthz to include `env!("CARGO_PKG_VERSION")` and the highest applied `schema_migrations.version` so the deployed build is identifiable (it is already unauthenticated and harmless — it exposes no user data). Add `add_header Strict-Transport-Security "max-age=31536000" always;` and `server_tokens off;` to server/nginx/family-connect.conf.

**F80 — 190 unit tests pass and lint is clean, but the 239 tests that actually exercise the server have never run automatically**  
*process · nice-to-have*

> **Evidence.** `cd server && cargo test`: 190 passed, 0 failed, 239 ignored, across 23 test binaries. Every ignored test is a real end-to-end suite gated on a live database — e.g. ws_flow.rs's 12 tests all read "ignored, needs a reachable PostgreSQL server; run with --ignored", including `deleting_an_account_fans_out_the_tombstone_and_closes_its_sockets` and `offline_members_with_push_tokens_reach_the_push_seam`; call_flow.rs, push_flow.rs, attachment_flow.rs, retention_flow.rs, account_flow.rs and migration_flow.rs are all in that set. `cargo clippy --all-targets -- -D warnings` exits 0 and `cargo fmt --check` exits 0. There is no CI: /Users/nettrash/Develop/nettrash.me/family.connect/.github has no workflows.

> **Fix.** Run `cargo test -- --ignored` against a throwaway PostgreSQL (docker is available on this machine) once before the release and record the result, then add a GitHub Actions workflow with a `postgres` service container running `cargo test -- --include-ignored`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` on every push.

**F81 — If [ai] is enabled on the production box, member text leaves to Azure OpenAI — unverifiable from here and undisclosed**  
*server · should-fix*

> **Evidence.** The assistant is a shipped feature gated entirely on server config: `GET /families/mine` only emits the `assistant` object when `handlers_ai::assistant_user_id(&state)` resolves (/Users/nettrash/Develop/nettrash.me/family.connect/server/src/handlers_family.rs:492-497), which is why the client hides it cleanly when it is off — good design. When on, questions go to an Azure OpenAI deployment with the system prompt plus up to `history_messages` turns (src/ai.rs, src/handlers_ai.rs, 1906 lines), configured by a `[ai]` block that ships commented out (config.example.toml). Whether it is enabled on fc.nettrash.me cannot be determined without an authenticated `GET /families/mine`. Meanwhile ios/docs/appstore.md claims "no third-party SDKs of any kind" and nettrash.me hosts no privacy policy for Family Connect at all (no familyconnect assets under frontend/assets/appstore/ or /play/).

> **Fix.** Determine whether `[ai]` is filled in on /etc/family-connect/config.toml. If it is, disclose the Azure OpenAI transfer in the privacy policy page that still has to be written for nettrash.me, answer the App Privacy questionnaire accordingly, and drop the "no third-party SDKs of any kind" line from ios/docs/appstore.md. If it is not, confirm the client hides the assistant entirely on the store build so no dead affordance ships.

### `ios-first-run-ux` (9)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F83 | Review notes ship with unfilled demo-account placeholders; nothing past the sign-in screen is reachable without them | process | should-fix |
| F84 | No iOS or iPad screenshots exist anywhere in the repo, and iPad screenshots are mandatory at family "1,2" | ios | should-fix |
| F85 | iPad gets a full-width stretched iPhone layout — no split view, no readable column, no bubble width cap | ios | should-fix |
| F86 | No way to block a member or report content, in a messaging app whose default server has open registration | ios | should-fix |
| F87 | "Change server…" has no Cancel — the review notes invite the reviewer into a screen they cannot back out of | ios | nice-to-have |
| F88 | Review notes and description tell App Review things the shipped binary contradicts (text-only, no third-party SDKs) | process | should-fix |
| F89 | ITSAppUsesNonExemptEncryption is absent from the built Info.plist — every upload parks in "Missing Compliance" | ios | nice-to-have |
| F90 | All five permission usage descriptions are English-only across nine declared localizations | ios | should-fix |
| F91 | Sharing into the app while signed out stages files and then silently swallows them | ios | nice-to-have |

**F83 — Review notes ship with unfilled demo-account placeholders; nothing past the sign-in screen is reachable without them**  
*process · should-fix*

> **Evidence.** ios/docs/appstore.md:49 `- Owner: username [DEMO_USER], password [DEMO_PASS].`; :57 `join the reviewer family with invite code [INVITE_CODE]`; :70 `contact [SUPPORT_EMAIL]`; :97-:100 the pre-submission checklist items for exactly these are all still unticked `- [ ]`. Verified against the binary: `RootView.swift:52-63` is a hard phase switch — `.needsAuth` renders `AuthView` and nothing else; there is no guest mode, no read-only preview, and `AuthView.swift:118` offers only Log In / Register / Change server. Every shipped feature (chats, photos, albums, voice messages, polls, board, calls) sits behind `session.phase == .active`.

> **Fix.** Create the two accounts on fc.nettrash.me, seed the reviewer family with members, message history and at least one 1:1 chat, and substitute the real values for [DEMO_USER]/[DEMO_PASS], [DEMO_USER_2]/[DEMO_PASS_2], [INVITE_CODE] and [SUPPORT_EMAIL] in ios/docs/appstore.md, then paste the owner credentials into App Store Connect → App Review Information. Also delete the stale `BLOCKER before submitting` block at ios/docs/appstore.md:5-8 — account deletion IS implemented (Views/DeleteAccountView.swift, reachable at SettingsView.swift:350).

**F84 — No iOS or iPad screenshots exist anywhere in the repo, and iPad screenshots are mandatory at family "1,2"**  
*ios · should-fix*

> **Evidence.** `find /Users/nettrash/Develop/nettrash.me/family.connect -iname "*screenshot*"` returns exactly one hit: `android/fastlane/metadata/android/en-US/images/phoneScreenshots`. There is no fastlane/, no appstore/ image dir and no screenshot asset under ios/. Meanwhile the built binary declares iPad: `plutil -p FC-ios.xcarchive/Products/Applications/FamilyConnect.app/Info.plist` → `UIDeviceFamily => [1, 2]` and `UISupportedInterfaceOrientations~ipad => [Portrait, PortraitUpsideDown, LandscapeLeft, LandscapeRight]`.

> **Fix.** Capture and upload at least the 6.9" iPhone set and the 13" iPad set. Or, if iPad is dropped (see the ipad-stretched-iphone-layout finding), set TARGETED_DEVICE_FAMILY = "1" and only the iPhone set is required.

**F85 — iPad gets a full-width stretched iPhone layout — no split view, no readable column, no bubble width cap**  
*ios · should-fix*

> **Evidence.** Screenshot proof at `/private/tmp/claude-501/-Users-nettrash-Develop-nettrash-me/fb26281e-02e2-4a33-b709-b88acf252442/scratchpad/firstrun-ipad-portrait.png` (iPad Pro 13", Release-nettrash build): the Log In/Register segmented control spans the entire 2064 px width, the Username/Password fields run edge to edge, and roughly 60 % of the screen below is empty. The cause is explicit in the source: `FamilyConnect/Core/PlatformStyle.swift:92-99` — `func setupColumn()` is `#if os(iOS) self` with the comment "No-op on iOS, where the window IS the column", which is true of a phone and false of a 13-inch iPad; RootView.swift:51-63 applies it to all five pre-chat screens. Past login it is the same story: `Views/ChatListView.swift:78` is `NavigationStack(path: $path)` — the Mac twin at `MacViews/MacChatView.swift:51` is a `NavigationSplitView`, and there is no iOS equivalent. `grep -n "maxWidth" Views/MessageBubbleView.swift` returns nothing, so a long message balloon has no width cap and will run the full iPad width.

> **Fix.** Either (a) make setupColumn() clamp on iOS too (`frame(maxWidth: 460)` when horizontalSizeClass == .regular), move ChatListView to a NavigationSplitView on regular width mirroring MacChatView, and cap balloon width at ~0.75 of the container; or (b) accept the trade-off and set TARGETED_DEVICE_FAMILY = "1" — the app then ships iPhone-only, iPad screenshots stop being required, and it still runs on iPad in compatibility mode. (b) is far cheaper but forfeits the iPad store presence.

**F86 — No way to block a member or report content, in a messaging app whose default server has open registration**  
*ios · should-fix*

> **Evidence.** `grep -rn -i "block|report abuse|mute user" FamilyConnect/Views/FamilyManageView.swift FamilyConnect/Views/SettingsView.swift` returns nothing, and `grep -i "block|report|abuse|moderat" docs/protocol.md` finds no moderation surface anywhere in the wire contract. The only moderation primitive is owner-only member removal (SettingsView.swift:289-297 → FamilyManageView). Meanwhile ios/docs/appstore.md:53 states "Registration is also fully open: tap Register and create any username and password" against the developer-operated fc.nettrash.me, and appstore.md:101 already flags this: "any App Store user can register and create their own family on your box".

> **Fix.** Add a per-member block (hides that member's messages locally and refuses their 1:1 chat) and a "Report" action on a message plus on a member, both reachable from the message context menu and the member row; state the retention/response commitment and a contact address in the review notes. If you would rather argue the exemption, at minimum close registration on fc.nettrash.me behind an invite (the `[registration]` server switch appstore.md:101 already proposes) and say so explicitly in the review notes.

**F87 — "Change server…" has no Cancel — the review notes invite the reviewer into a screen they cannot back out of**  
*ios · nice-to-have*

> **Evidence.** ios/docs/appstore.md:56 tells the reviewer: "The 'Change server' footer on that screen is where self-hosted servers are entered; any URL can be tried there and reverted." There is no revert affordance. `AuthView.swift:107` calls `session.requestServerChange()`, which is `AppSession.swift` → `phase = .needsServer`; `RootView.swift:53-54` then renders `ServerSetupView()` as the ENTIRE root — it is a phase switch, not a pushed screen. `grep -c toolbar FamilyConnect/Views/ServerSetupView.swift` = 0: the view has two Form sections, a Connect button and no Cancel, no back, no dismiss. Its only escape is `session.setServer(url)` succeeding.

> **Fix.** Add a cancellation toolbar item to ServerSetupView shown only when AppSettings.serverURL is already set — `Button("Cancel") { session.cancelServerChange() }` returning phase to .needsAuth — and reword appstore.md:56 to describe the actual recovery. A one-line "Back to sign in" button in the footer would do.

**F88 — Review notes and description tell App Review things the shipped binary contradicts (text-only, no third-party SDKs)**  
*process · should-fix*

> **Evidence.** ios/docs/appstore.md:31 "What it does not have: ads, analytics, tracking, or third-party SDKs of any kind. And the honest limits of version 1: text messages only. Voice and video calls are planned." and :67 "- No third-party SDKs, no analytics..." and :70 "- Text messages only in this version." The binary disagrees on both counts: the project has exactly one SPM dependency, the stasel/WebRTC 151.0.0 binary XCFramework, and the built Info.plist declares `UIBackgroundModes => [audio, voip]`, `NSCameraUsageDescription`, `NSMicrophoneUsageDescription`, `NSLocationWhenInUseUsageDescription`, `NSPhotoLibraryAddUsageDescription` and `NSLocalNetworkUsageDescription` (all confirmed via plutil on FC-ios.xcarchive/Products/Applications/FamilyConnect.app/Info.plist). Views/ present: AttachmentViewer, AudioPlayerView, PollComposerView, LocationAttachmentView, CallView, CallVideoView.

> **Fix.** Rewrite the Description, Beta App Description and Notes for App Review to the actual v1 feature set (photos/videos/albums, voice messages, files, polls, location sharing, reactions, board, edits, share extension, WebRTC voice AND video calls with CallKit), state the one third-party dependency by name and licence, and redo the App Privacy label to declare Photos or Videos, Audio Data, Coarse/Precise Location and User Content alongside User ID.

**F89 — ITSAppUsesNonExemptEncryption is absent from the built Info.plist — every upload parks in "Missing Compliance"**  
*ios · nice-to-have*

> **Evidence.** `plutil -p /private/tmp/.../FC-ios.xcarchive/Products/Applications/FamilyConnect.app/Info.plist | grep -i ITSAppUsesNonExemptEncryption` returns nothing; the key is neither in ios/FamilyConnect/Info.plist (which carries only NSAppTransportSecurity, UIBackgroundModes, NSUserActivityTypes, INIntentsSupported, FCDefaultServerURL, CFBundleURLTypes) nor as an INFOPLIST_KEY_* build setting. The app does use encryption — HTTPS to the server plus DTLS-SRTP inside WebRTC.

> **Fix.** Add `INFOPLIST_KEY_ITSAppUsesNonExemptEncryption = NO` (the app uses only standard HTTPS/DTLS for its own purposes, which is exempt) to the shared build settings, or add `<key>ITSAppUsesNonExemptEncryption</key><false/>` to ios/FamilyConnect/Info.plist beside the other non-INFOPLIST_KEY entries.

**F90 — All five permission usage descriptions are English-only across nine declared localizations**  
*ios · should-fix*

> **Evidence.** `find FC-ios.xcarchive/Products/Applications/FamilyConnect.app -name "InfoPlist.strings"` returns nothing; the shipped ru.lproj contains only `AppIntentVocabulary.plist`, `Localizable.strings`, `Localizable.stringsdict`. There is no InfoPlist.xcstrings anywhere in the repo (`find . -name "*.xcstrings"` returns exactly one file, FamilyConnect/Localizable.xcstrings). Yet the bundle ships Base/en/de/es/fr/ja/ru/sr/sr-Latn/zh-Hans and the app UI is fully translated (437/437 keys `translated` in every non-English language, verified by script).

> **Fix.** Add an InfoPlist.xcstrings to the app target and translate NSCameraUsageDescription, NSMicrophoneUsageDescription, NSLocationWhenInUseUsageDescription, NSPhotoLibraryAddUsageDescription and NSLocalNetworkUsageDescription into the eight non-English languages, matching the quality of Localizable.xcstrings.

**F91 — Sharing into the app while signed out stages files and then silently swallows them**  
*ios · nice-to-have*

> **Evidence.** FamilyConnectShareExtension/ShareViewController.swift:96-113 (`handleShare`) stages every item into the App Group inbox and calls `openHostApp(url)` with no session check of any kind — it never reads the keychain or AppSettings. RootView.swift:91-93 routes the URL to `AppSession.handleShareURL`, which parks the moved files in `pendingShareImport` (Core/AppSession.swift). The only consumer is ChatListView.swift:226/:231-:239, which is reachable exclusively at `phase == .active`. At `.needsAuth` the app therefore opens on the sign-in screen with no banner, no toast and no mention that anything was shared.

> **Fix.** Have the share extension check for a stored session (server URL + keychain token via the App Group) and, when absent, show "Sign in to Family Connect first" and complete without staging; or keep staging and surface a one-line banner on AuthView/FamilyGateView ("1 file waiting — sign in to send it") driven by `session.pendingShareImport != nil`.

### `macos-app-quality` (5)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F95 | Sandbox: "Save…" in the Mac attachment viewer silently does nothing (read-only user-selected entitlement) | macos | should-fix |
| F96 | No macOS documentation anywhere: appstore.md, README and CHANGELOG never mention the Mac app at all | macos | should-fix |
| F97 | ENABLE_HARDENED_RUNTIME is not set in any build configuration | macos | nice-to-have |
| F98 | No Settings scene: ⌘, does nothing and there is no Settings item in the app menu | macos | nice-to-have |
| F99 | No drag-and-drop of files into a chat anywhere in the app | macos | nice-to-have |

**F95 — Sandbox: "Save…" in the Mac attachment viewer silently does nothing (read-only user-selected entitlement)**  
*macos · should-fix*

> **Evidence.** ios/FamilyConnect/FamilyConnect-macOS.entitlements declares ONLY `com.apple.security.files.user-selected.read-only` (no `...read-write`), and its own comment says "user-selected.read-only covers the document picker". But ios/FamilyConnect/MacViews/MacAttachmentViewer.swift:150-168 writes to a user-chosen destination:
```swift
let panel = NSSavePanel()
...
guard panel.runModal() == .OK, let destination = panel.url else { return }
try? FileManager.default.removeItem(at: destination)
try? FileManager.default.copyItem(at: source, to: destination)
```
Apple's App Sandbox entitlement reference is explicit: `com.apple.security.files.user-selected.read-write` is what "enables read/write access to files the user selects using an Open or Save dialog"; the read-only variant grants read access only. Both write calls here are `try?`, so the EPERM is swallowed with no error, no log, and no UI. The Save… toolbar button (MacAttachmentViewer.swift:83-90) is the only way to get a received photo or video out of the app on macOS.

> **Fix.** Add `<key>com.apple.security.files.user-selected.read-write</key><true/>` to ios/FamilyConnect/FamilyConnect-macOS.entitlements (replacing or in addition to the read-only key), and stop swallowing the failure: replace the two `try?` calls in MacAttachmentViewer.save() with a `do/catch` that surfaces a failure notice the way the conversation view's `mediaNotice` does, so a future sandbox regression is visible instead of silent.

**F96 — No macOS documentation anywhere: appstore.md, README and CHANGELOG never mention the Mac app at all**  
*macos · should-fix*

> **Evidence.** `grep -rn -i "macos|mac app|desktop| Mac " README.md CHANGELOG ios/docs/appstore.md` in /Users/nettrash/Develop/nettrash.me/family.connect returns ZERO hits, and `ls ios/docs/` shows appstore.md is the only doc. ios/docs/appstore.md (102 lines) is written entirely for iOS — its review notes name demo accounts and say "Signing in with this account on a second device…" (line 50) with no mention that a second, separately-reviewed Mac binary exists. Meanwhile the Mac build is a full product: five window scenes in FamilyConnectApp.swift, ~200KB of Mac-only views, and a "Mac Team Store Provisioning Profile: me.nettrash.FamilyConnect" already issued with aps-environment=production.

> **Fix.** Add a macOS section to ios/docs/appstore.md (or a sibling ios/docs/appstore-macos.md) covering: (a) the Mac listing copy and screenshot plan; (b) review notes stating that the Mac rings incoming calls through a Notification Center banner with Answer/Decline rather than CallKit, and that notification permission must be granted for calls to be announced; (c) the deliberate macOS differences (file picker instead of Photos picker, no in-app capture, Copy-via-context-menu instead of text selection). Also correct the already-stale iOS claims flagged elsewhere in this audit, and add the Mac app to README.md and CHANGELOG.

**F97 — ENABLE_HARDENED_RUNTIME is not set in any build configuration**  
*macos · nice-to-have*

> **Evidence.** `grep -c "ENABLE_HARDENED_RUNTIME" ios/FamilyConnect.xcodeproj/project.pbxproj` → `0`. The setting appears in no configuration (Debug, Release, Release-nettrash) for any target, so it defaults to NO. The App Sandbox itself IS correctly configured — `com.apple.security.app-sandbox` is true in ios/FamilyConnect/FamilyConnect-macOS.entitlements and in ios/FamilyConnectShareExtension/FamilyConnectShareExtension.entitlements (the latter required for a Mac .appex) — and `CODE_SIGN_ENTITLEMENTS[sdk=macosx*]` correctly selects the Mac file (project.pbxproj:535, 588, 797).

> **Fix.** Set `ENABLE_HARDENED_RUNTIME = YES` for the app and share-extension targets in all three configurations of ios/FamilyConnect.xcodeproj/project.pbxproj (Xcode: Signing & Capabilities → + Hardened Runtime), then do one SIGNED macOS launch to confirm WebRTC.framework still loads and that microphone/camera capture in a call still works. If a runtime exception proves necessary for the framework, add it explicitly rather than leaving the capability off.

**F98 — No Settings scene: ⌘, does nothing and there is no Settings item in the app menu**  
*macos · nice-to-have*

> **Evidence.** The only menu-bar customization in the whole app is one command: `CommandGroup(after: .toolbar) { Button("Refresh") {...}.keyboardShortcut("r", modifiers: .command) }` (ios/FamilyConnect/FamilyConnectApp.swift:261-266). `grep -rn "CommandGroup|CommandMenu|Settings {"` finds no `Settings { }` scene anywhere. Settings is instead a `.sheet(isPresented: $showingSettings) { MacSettingsView() }` opened from the main window's toolbar (MacChatView.swift:136-146, 149-151), and MacSettingsView.swift:7-10 states the choice deliberately: "A sheet rather than a Settings scene". SwiftUI inserts the "Settings…" item into the app menu only when a Settings scene exists, so as built the app menu has no Settings item and ⌘, is unbound. MacSettingsView is additionally pinned at `.frame(width: 460, height: 530)` (line 146) — a Mac sheet cannot be resized — and it is unreachable from a standalone conversation window, the Board window, the Call window, or the attachment viewer.

> **Fix.** Add a `Settings { MacSettingsView() }` scene to FamilyConnectApp.body (SwiftUI binds ⌘, and adds the app-menu item automatically) and let the existing toolbar button call `openSettings()` from `@Environment(\.openSettings)` instead of raising a sheet. A Settings scene is a real resizable window, which also removes the fixed 460×530 clipping risk the file's own comment describes. While there, consider adding menu commands for the Board and Call windows — both are currently reachable only from the main window's toolbar (MacChatView.swift:116-134).

**F99 — No drag-and-drop of files into a chat anywhere in the app**  
*macos · nice-to-have*

> **Evidence.** `grep -rn "onDrop|dropDestination|NSFilePromise|registerForDraggedTypes|\.draggable(" --include="*.swift" ios/FamilyConnect` returns zero matches across the entire codebase. Attaching a file on Mac is only possible through the modal NSOpenPanel (MacViews/MacFilePicker.swift:23-48, invoked from MacConversationView.pickAttachment) or ⌘V paste (MacConversationView.swift:312-325). Conversely, there is no way to drag an attachment OUT of a message to the Finder either.

> **Fix.** Add `.dropDestination(for: URL.self)` to the MacConversationView message list / composer area, feeding the dropped URLs into the same `MediaPrep.prepareAny` staging path that `MacFilePicker.pickMany()` already feeds (MacConversationView.swift:1560+), honouring the same ten-attachment cap. Optionally add `.draggable` on attachment tiles in MacMessageRow so a photo can be dragged to the Finder — which would also give users a working export route independent of the save-panel entitlement.

### `release-hygiene` (10)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F103 | No privacy policy or support page exists anywhere — both are required App Store Connect fields | both | blocker |
| F104 | ios/docs/appstore.md — the only source for the Description and Review Notes — is factually wrong about the shipping app on four counts | both | should-fix |
| F105 | CHANGELOG has never been touched since the first commit — 77 commits of user-visible features are unrecorded | process | nice-to-have |
| F106 | Version-number question: the first App Store release is 1.0, and the CHANGELOG should say v1.0 not v1.0.0 | process | nice-to-have |
| F107 | Working tree is dirty: CURRENT_PROJECT_VERSION was bumped 101→103 by the archive PostAction and never committed | both | nice-to-have |
| F108 | README says push notifications send nothing — the server has a full APNs + VoIP + FCM implementation | process | nice-to-have |
| F109 | README's headline privacy claim "no third party ever sees a message" is not true when the optional assistant is configured | process | nice-to-have |
| F110 | The shipped binary redistributes Google's WebRTC with no notice anywhere in the app or the listing | both | should-fix |
| F111 | A branch is named `v1.0` locally and on origin — a `v1.0` tag would make every reference to that name ambiguous | process | nice-to-have |
| F112 | Zero CI: nothing has built or tested any of the 78 commits before an archive is cut | process | nice-to-have |

**F103 — No privacy policy or support page exists anywhere — both are required App Store Connect fields**  
*both · blocker*

> **Evidence.** `find . -iname "*privacy*" -o -iname "*support*"` over the whole repo returns exactly one hit: `ios/FamilyConnect/PrivacyInfo.xcprivacy` (the privacy manifest, which is not a policy). There is no PRIVACY.md, no support doc, and `docs/` contains only `protocol.md` while `ios/docs/` contains only `appstore.md`. The nettrash.me site repo has store/legal pages for exchange, geo, md and scan under `frontend/assets/appstore/` but zero hits for "familyconnect"/"family connect". ios/docs/appstore.md:100 still carries this as an unticked checklist item: "- [ ] Fill `[SUPPORT_EMAIL]`; set Support URL and Privacy Policy URL (a page on nettrash.me explaining both modes…)".

> **Fix.** Write two pages on nettrash.me under `frontend/assets/appstore/familyconnect/` (privacy.html, support.html) following the pattern the four existing apps use, and add the Family Connect card to home.rs. The privacy text must distinguish the two modes explicitly: (a) default server — the developer operates fc.nettrash.me and stores username, messages, attachments and device push tokens there; (b) self-hosted — nothing reaches the developer at all. It must also disclose the optional assistant (off by default; when an operator enables it, that member's own thread is sent to the configured Azure OpenAI endpoint — see server/config.example.toml lines 225-290 and docs/protocol.md:460). Then fill both URLs plus the support email in App Store Connect.

**F104 — ios/docs/appstore.md — the only source for the Description and Review Notes — is factually wrong about the shipping app on four counts**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md:31 (Description, the text that goes in the store): "What it does not have: ads, analytics, tracking, or third-party SDKs of any kind. And the honest limits of version 1: text messages only. Voice and video calls are planned." — but the app ships photos/videos/voice messages/files/locations (server/migrations/0009_attachment*.sql through 0025_attachment_sets.sql; ios/FamilyConnect/Views/AttachmentViewer.swift), P2P voice AND video calls (server/src/handlers_call.rs, server/migrations/0024_calls.sql + 0026_call_video.sql, ios/FamilyConnect/Core/Calls/WebRTCClient.swift), and exactly one third-party SDK — `ios/FamilyConnect.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved` pins `https://github.com/stasel/WebRTC.git` at version 151.0.0, revision 19aa8c1f. appstore.md:66 (Review Notes) "- Text messages only in this version." appstore.md:80 (TestFlight) "What is not here yet, on purpose: photos and other media, and voice/video calls." appstore.md:5-8 still opens with "**BLOCKER before submitting** (guideline 5.1.1(v))… v1 does not have one yet" and appstore.md:96 still has "- [ ] Implement account deletion" unticked — both false: server/src/app.rs:41 routes `POST /api/v1/me/delete` to handlers_auth::delete_account, with ios/FamilyConnect/Views/DeleteAccountView.swift and ios/FamilyConnectTests/AccountDeletionTests.swift.

> **Fix.** Rewrite ios/docs/appstore.md against the shipped feature set before touching App Store Connect. Concretely: delete the blocker banner at lines 5-8 and tick line 96 (point it at `POST /api/v1/me/delete` + Settings → Account → Delete Account, which is what the notes already describe at line 74); rewrite the Description feature list at lines 25-33 to cover media, albums, voice messages, files, locations, polls, reactions, replies, edits, the family board, the share extension and P2P voice/video calls; replace "no third-party SDKs of any kind" with the accurate statement (one binary dependency, Google's WebRTC via stasel/WebRTC 151.0.0, used only for the peer-to-peer call media path, with no analytics or tracking SDK); rewrite the Review Notes (lines 60-70) and the HOW TO REVIEW script so it walks the reviewer through sending a photo, a voice message and placing a call between the two demo accounts, and explains the camera/microphone/notification prompts and CallKit; rewrite the TestFlight sections at lines 76-92; and add checklist items for the two store URLs and for the App Privacy answers to also cover attachments and call metadata, not just messages.

**F105 — CHANGELOG has never been touched since the first commit — 77 commits of user-visible features are unrecorded**  
*process · nice-to-have*

> **Evidence.** `git log --oneline -- CHANGELOG` returns exactly one commit: `18544d3 init`. `git log --oneline | wc -l` returns 78. The single entry (CHANGELOG:1-12) describes "push-notification hooks without a transport yet" and a text-only chat; since then the log shows `push apns fcm`, `attachements`, `macOS + Localizations`, `audio, fixes, ai`, `famili stat`, `reactions`, `reply`, `edit`, `board`, `votes + paste`, `voice p2p calls`, `p2p video calls`, `multiple attachements, share with FamilyConnect`, `Android calls`, and `photo ui improvements` (git log --format='%h %ad %s' --date=short).

> **Fix.** Prepend this entry to /Users/nettrash/Develop/nettrash.me/family.connect/CHANGELOG, followed by a blank line and then the existing v0.1.0 entry. Column layout matches the existing entry exactly: date in columns 1-10, ten spaces, then the version; body indented 20 spaces and wrapped so no line exceeds 69 characters.

2026-08-28          v1.0

                    Everything the clients ship beyond text. Media:
                    photos, videos, voice messages, files and shared
                    locations, up to ten attachments per message,
                    grouped into albums with a full-screen swipeable
                    viewer. Conversation: replies, message editing,
                    emoji reactions, polls with votes, link previews,
                    Markdown, paste-to-send, a first-unread divider
                    and app-icon badges. Family: profile pictures,
                    birthdays, a family language, family statistics,
                    and a shared board of sticky notes in six colours
                    and three sizes. Push notifications now really
                    deliver, over APNs (alert and VoIP) and FCM.
                    Peer-to-peer voice and video calls over WebRTC
                    with TURN, CallKit on iOS and Telecom on Android.
                    An optional per-family assistant, private to each
                    member and reachable as @ai in the family chat,
                    off unless the operator configures it. A native
                    macOS app (macOS 14+) built from the same target
                    as the iPhone and iPad app. Account deletion from
                    Settings. A Share Extension that sends anything
                    from another app straight into a chat. Nine more
                    languages: Russian, German, Spanish, French,
                    Japanese, Serbian in both scripts and Simplified
                    Chinese. Server-side attachment de-duplication
                    and retention sweeps.

**F106 — Version-number question: the first App Store release is 1.0, and the CHANGELOG should say v1.0 not v1.0.0**  
*process · nice-to-have*

> **Evidence.** `grep -n MARKETING_VERSION ios/FamilyConnect.xcodeproj/project.pbxproj` → `MARKETING_VERSION = 1.0;` at 12 sites (lines 566, 619, 644, 668, 691, 714, 828, 853, 876, 907, 941, 975) — every target × every configuration agrees, no dissenters. android/app/build.gradle.kts:48 defaults versionName to "1.0" as well. The CHANGELOG's only entry reads `v0.1.0` (CHANGELOG:1). ios/FamilyConnect/Core/AppVersion.swift:33 renders the Settings line as `1.0 (51)` style from CFBundleShortVersionString.

> **Fix.** Number the first App Store release **1.0**, not 1.0.0 and not 0.2.0: CFBundleShortVersionString is already 1.0 in all twelve configurations and in the shipped Settings line, Android's versionName defaults to the same, and that two-component string is what the App Store will display — the CHANGELOG is the only artefact that disagrees, so it is the one to change. v0.1.0 was a pre-store development entry that was never published anywhere, so nothing depends on the three-component style; write the new entry as `v1.0` (as in the fix above) so the file, both binaries and the store all read the same string. Do not raise MARKETING_VERSION to 1.0.0 — two components is valid for CFBundleShortVersionString and changing it now would invalidate the twelve configurations for no gain.

**F107 — Working tree is dirty: CURRENT_PROJECT_VERSION was bumped 101→103 by the archive PostAction and never committed**  
*both · nice-to-have*

> **Evidence.** `git status --porcelain` → ` M ios/FamilyConnect.xcodeproj/project.pbxproj` (the only modified file). `git diff` shows `-CURRENT_PROJECT_VERSION = 101;` / `+CURRENT_PROJECT_VERSION = 103;` at 12 sites — 12 insertions, 12 deletions. The bump script is a **Archive** PostAction, not a Build one: ios/FamilyConnect.xcodeproj/xcshareddata/xcschemes/FamilyConnect.xcscheme:98-101 is `<ArchiveAction buildConfiguration="Release"…><PostActions>` with `scriptText = "cd "${PROJECT_DIR}" ; agvtool bump"`, and FamilyConnect-nettrash.xcscheme:107-110 is the same on `Release-nettrash`. Every previous `release` commit is exactly this bump plus android/version.properties (e.g. `git show --stat 989334f` → 2 files, project.pbxproj 24 lines + version.properties 4 lines).

> **Fix.** Commit the pbxproj bump before archiving the build that will actually be uploaded, so the uploaded build number exists in history; or archive, then commit the resulting 10x value with the release commit, matching the existing `release`-commit pattern. Either way, do not archive again from a dirty tree — check `git status` is clean immediately before the archive that goes to App Store Connect. (Also worth knowing: `xcodebuild build` and `xcodebuild test` do NOT bump, because the PostAction hangs off ArchiveAction only; only an archive moves the number.)

**F108 — README says push notifications send nothing — the server has a full APNs + VoIP + FCM implementation**  
*process · nice-to-have*

> **Evidence.** README.md:28-30: "Push notifications are a hook in v1: device tokens are collected and delivery events logged, but nothing is sent until an APNs/FCM transport is configured in a later version." Contradicted by server/src/push.rs (863 lines): `ApnsSender::send_one` posts to APNs with `apns-topic`/`apns-push-type: alert`/`apns-priority: 10` headers (lines 309-317), `send_voip_one` posts with a `.voip` topic suffix and `apns-push-type: voip` (lines 386-401), `FcmSender::send_one` posts an FCM v1 message (lines 577-588), provider JWTs are minted at lines 261/505 and unregistered-token reaping at 445/698. The same README already documents the real credential setup at lines 108-124 (AuthKey_<KEYID>.p8, firebase-service-account.json, `[push.apns]`/`[push.fcm]`), so the file contradicts itself. docs/protocol.md has a matching stale artefact at line 1249: the section heading `### Devices (push hook — no delivery in v1)`, even though the section's own body describes real VoIP delivery and §Push notifications at line 1452 documents the live behaviour.

> **Fix.** Four README corrections, plus one in the protocol doc. README.md:5-7 — replace "Text messages in v1; the protocol is shaped so voice/video call signaling can be added later" with the real feature set (media, polls, board, and peer-to-peer voice/video calls over WebRTC). README.md:17 — the layout line says "ios/ # SwiftUI client, iOS 17+", but the single target also builds macOS 14+ (SUPPORTED_PLATFORMS includes macosx, MACOSX_DEPLOYMENT_TARGET = 14.0, ios/FamilyConnect/MacViews/ has 10 files); say "SwiftUI client, iOS 17+ and macOS 14+". README.md:28-30 — replace the hook sentence with "Push notifications are delivered over APNs (alert and PushKit VoIP for incoming calls) and FCM once the operator supplies the credentials described under Installing the server; without them, would-be notifications are only logged and everything else works unchanged." README.md:13-20 — the layout tree omits `tools/` and `CODEOWNERS`, which exist. docs/protocol.md:1249 — change the heading to `### Devices` (drop "push hook — no delivery in v1").

**F109 — README's headline privacy claim "no third party ever sees a message" is not true when the optional assistant is configured**  
*process · nice-to-have*

> **Evidence.** README.md:3-4: "the iOS and Android apps talk only to it, and no third party ever sees a message." But server/config.example.toml:234-290 configures an Azure OpenAI `endpoint`, `deployment` and `api_key`, and docs/protocol.md:460-480 documents that a member's assistant thread — and, when the owner turns on `ai_history`, recent family-chat history — is sent to that endpoint. The README's "Security notes" section (README.md:128-137) does not mention the assistant at all, and neither does the repo-layout description.

> **Fix.** Qualify it in README.md:3-4 — "no third party sees a message" is true of the base product, so say so and name the one exception: add to the Security notes section that the optional AI assistant is off unless the operator configures an Azure OpenAI endpoint, and that when it is on, only the asking member's own assistant thread leaves the server (plus recent family-chat history if the owner enables `ai_history`), never another member's words. protocol.md already states this invariant precisely at lines 468-471; borrow its wording.

**F110 — The shipped binary redistributes Google's WebRTC with no notice anywhere in the app or the listing**  
*both · should-fix*

> **Evidence.** Package.resolved pins `https://github.com/stasel/WebRTC.git` 151.0.0 (revision 19aa8c1f) — a prebuilt XCFramework of Google's WebRTC, embedded in the .app and therefore in the .ipa. A grep of ios/FamilyConnect/**/*.swift for "acknowledg", "open source", "third-party", "licen[cs]e" and "BSD" returns only three unrelated prose comments (AvatarStore.swift:13, AppSettings.swift:34, UnreadAnchor.swift:61); a grep of Localizable.xcstrings for any string containing "acknowledg", "open source" or "licen[cs]e" returns nothing. ios/FamilyConnect/Views/ has no About or Acknowledgements screen — Settings shows only the version line (ios/FamilyConnect/Views/SettingsView.swift:357 → AppVersion.settingsLine, "Family 1.0 (103)"). The repo's only LICENSE is the project's own MIT.

> **Fix.** Presenting this as an option with the facts, since the workspace owner has already decided against a notices file for md's bundled engines and that decision may well apply here too. Three ways to satisfy it, in increasing effort: (1) put the WebRTC copyright notice and BSD text on the Family Connect support page on nettrash.me — the page you have to write anyway for the store's Support URL — which is "other materials provided with the distribution" and costs nothing extra; (2) add a plain "Acknowledgements" row at the bottom of SettingsView (below the version line at SettingsView.swift:357) and MacSettingsView.swift:124, opening a static text view — the conventional Apple-ecosystem answer, ~40 lines, and the string is not translatable so it adds nothing to the ten .lproj sets; (3) do nothing, which is the current state. Option (1) is the cheapest and is the one that pairs with a decision to keep the app itself free of a notices screen.

**F111 — A branch is named `v1.0` locally and on origin — a `v1.0` tag would make every reference to that name ambiguous**  
*process · nice-to-have*

> **Evidence.** `git branch -avv` shows the session is on `* v1.0  989334f [origin/v1.0] release` alongside `master 989334f [origin/master]` and `remotes/origin/v1.0`. `git tag` returns nothing — no release tag exists yet. All three of master, v1.0 and origin/master point at the same commit 989334f, and `git rev-list --left-right --count origin/master...HEAD` → `0 0`, so everything is pushed and in sync.

> **Fix.** Decide the naming before the release, not after. Either rename the branch (`git branch -m v1.0 release/1.0` plus the matching push/delete on origin) and keep `v1.0` free for the tag, or tag as `1.0` without the `v` and accept that the CHANGELOG's `v` prefix is a document convention rather than a ref name. Tagging itself is the owner's to do — this is only about clearing the name first. Separately: `remotes/origin/main` still points at `18544d3 init`, 78 commits behind, while `origin/HEAD -> origin/master` — a stale branch on a public repo that shows an empty skeleton to anyone who lands on it by URL; delete it or fast-forward it.

**F112 — Zero CI: nothing has built or tested any of the 78 commits before an archive is cut**  
*process · nice-to-have*

> **Evidence.** `git ls-files .github` returns nothing — the `.github/` directory on disk is entirely untracked and self-ignored (`.github/modernize/java-upgrade/.gitignore` contains `**/*`, and `git status --ignored` collapses the whole tree to `!! .github/`). There are no workflow files in the repository at all. Meanwhile the repo carries substantial test suites that nothing runs automatically: server/tests/ has 10+ integration flows (chat_flow, push_flow, board_flow, poll_flow, assistant_flow, family_flow, birthday_flow, migration_flow, edit_flow), ios/FamilyConnectTests/ has 20+ test files, and android/ has JVM tests.

> **Fix.** Add a minimal `.github/workflows/ci.yml` running three jobs on push and PR: `cd server && cargo build --release && cargo fmt --all -- --check && cargo clippy --all-targets && cargo test` (the `--ignored` integration tests need a Postgres service container, worth adding); `cd android && ./gradlew assembleStandardDebug testStandardDebugUnitTest lintStandardDebug` on JDK 21; and on macos-latest `xcodebuild test -project ios/FamilyConnect.xcodeproj -scheme FamilyConnect -destination 'platform=iOS Simulator,name=iPhone 16' CODE_SIGNING_ALLOWED=NO`. Note the iOS job must use the `FamilyConnect` scheme and the `test` action, never `archive` — an archive would fire the agvtool PostAction and dirty the checkout.

### `code-robustness` (7)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F115 | One transient network error permanently blanks a photo for the rest of the app session | both | should-fix |
| F116 | A composed media message exists nowhere until every upload succeeds, and nothing keeps the app alive to finish them | ios | should-fix |
| F117 | SwiftData store has no schema version and no recovery path — a container failure is a permanent "reinstall the app" dead end | both | should-fix |
| F118 | Reconnect backoff resets on handshake, so a proxy that accepts the upgrade and drops it produces a ~2 Hz reconnect + full-resync loop | both | should-fix |
| F119 | Opening or sharing a file/video attachment buffers the entire file (up to the 100 MB protocol limit) in memory | both | nice-to-have |
| F120 | If a CallKit transaction is refused, the in-app End button does nothing and the call hangs until a 90-second guard | ios | should-fix |
| F121 | The PushKit handler runs a synchronous SwiftData fetch before reporting the incoming call to CallKit | ios | nice-to-have |

**F115 — One transient network error permanently blanks a photo for the rest of the app session**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/Core/AttachmentStore.swift:68 `guard !missing.contains(key) else { return nil }` gates before any retry; :92-99 in `fetch(id:preview:key:)` the failure branch is `guard let data, let decoded = PlatformImage.decode(...) else { missing.insert(key); return }` — reached identically for a 404, a timeout, a refused connection and a 5xx, because `try? await api.attachmentData(...)` flattens both `nil` (404) and `throw` (transport) to `nil`. Nothing ever removes a key from `missing` except `clear()` (:161, logout only) and `forget(attachmentIDs:)` (:145). `generation` is not even bumped on that path, so no view re-renders. The sibling store proves this is a known-wrong design: AvatarStore.swift:96/:100/:107 passes an explicit `settled:` flag — `true` only for a real 404 — and :124 `} else if settled {` is the only path that inserts into `missing`, with the comment "Marking it missing here is what would freeze a whole roster on initials after one bad minute on cellular, with nothing to undo it but a relaunch."

> **Fix.** Give AttachmentStore the `settled` flag AvatarStore already has: catch `APIError.notFound` (or a nil return) as settled → insert into `missing`; treat every other error as unsettled → do not insert, and bump `generation` so the view retries on the next render pass. Same file, same shape as AvatarStore.swift:88-129.

**F116 — A composed media message exists nowhere until every upload succeeds, and nothing keeps the app alive to finish them**  
*ios · should-fix*

> **Evidence.** ios/FamilyConnect/Core/ChatSyncCoordinator.swift:1420-1480 — `sendMedia(_:caption:replyTo:in:onItemStart:)` runs the whole `for (index, item) in prepared.enumerated()` upload loop first and only reaches `guard let localID = enqueue(body: caption, in: chatID, replyTo: replyTo, allowEmpty: true)` at :1480. Text takes the opposite order (:1295 `enqueue` then `pendingDelivery = Task { deliver }`), which is what makes `sweepOutbox()` (:2063) able to re-send a pending row after a relaunch. ios/FamilyConnect/Views/ConversationView.swift:1769 `sendStaged` calls `takeComposer()` and `staged = []` before starting the unstructured `Task`, so the caption, the reply target and the chips are all out of the view's state during the upload. Uploads go through `APIClient` on `URLSession.shared` (APIClient.swift:66 `init(serverURL:session: URLSession = .shared)`) — `grep -rn "beginBackgroundTask|URLSessionConfiguration.background" FamilyConnect FamilyConnectShareExtension` returns nothing, and `plutil -p FamilyConnect/Info.plist` shows `UIBackgroundModes => [audio, voip]` only.

> **Fix.** Enqueue the pending row (and stage the attachment ids on it) BEFORE uploading, so a killed or backgrounded send is a `.pending`/`.failed` row the existing `retry(localID:)` and `sweepOutbox()` machinery can finish; and wrap the upload in `UIApplication.beginBackgroundTask` (or move attachment uploads to a background `URLSessionConfiguration.background` session) so a normal screen lock does not kill it.

**F117 — SwiftData store has no schema version and no recovery path — a container failure is a permanent "reinstall the app" dead end**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/FamilyConnectApp.swift:86 `try ModelContainer(for: schema, configurations: [configuration])` — no `migrationPlan:` argument, and `grep -rn "VersionedSchema|SchemaMigrationPlan|migrationPlan" FamilyConnect` returns zero hits across all four `@Model` types (ChatEntity, MessageEntity, MemberEntity, NoteEntity). On failure the scene renders `StoreErrorView` (:245, defined :364) whose only guidance is :382 `Text("Reinstall Family Connect to start fresh. Your messages are safe on the family server and will re-download.")`. Nothing deletes the store file and retries.

> **Fix.** Two changes in FamilyConnectApp.init: (1) wrap the schema in a `VersionedSchema` (`FamilyConnectSchemaV1`) and pass a `SchemaMigrationPlan`, so release 2 has somewhere to attach a migration stage; (2) on a container failure, delete the store files at `configuration.url` and retry `ModelContainer(...)` once before falling back to StoreErrorView — keep StoreErrorView only for the second failure.

**F118 — Reconnect backoff resets on handshake, so a proxy that accepts the upgrade and drops it produces a ~2 Hz reconnect + full-resync loop**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/Core/ChatSocket.swift:179-183 — inside `runLoop()`, the handshake ping succeeding is treated as success: `isConnected = true; lastAliveAt = Date(); backoff.reset(); continuation.yield(.connected); startHeartbeat(task); try await receiveLoop(...)`. If `receiveLoop` throws on the next line, the loop falls through to `let delay = backoff.nextDelay()` with `attempt == 0`, i.e. `random(0...1)` seconds — forever, with no escalation. Each `.connected` yield drives ios/FamilyConnect/Core/ChatSyncCoordinator.swift:413-415 `case .connected: connectionState = .connected; Task { await self.resync() }`, and `resync()` is a full sweep (chats, per-chat catch-up, edits, reactions, polls, board, outbox).

> **Fix.** Only reset the backoff once a connection has proven durable — e.g. record `connectedAt` at the handshake and call `backoff.reset()` from `receiveLoop` on the first successfully decoded frame, or after `Date().timeIntervalSince(connectedAt) > 10` at teardown. Leave the pre-connect `backoff.reset()` in `start()`/`resume()` (:97, :149) as it is.

**F119 — Opening or sharing a file/video attachment buffers the entire file (up to the 100 MB protocol limit) in memory**  
*both · nice-to-have*

> **Evidence.** ios/FamilyConnect/Core/ChatSyncCoordinator.swift:1525-1534 in `localFileURL(for:)`: `guard let data = try? await api.attachmentData(id: attachment.id, preview: false) ?? nil else { return nil }` then `try data.write(to: destination, options: .atomic)`. `APIClient.attachmentData` (APIClient.swift:570-578) goes through `perform` → `session.data(for: request)`, which materialises the whole body as `Data`. The ceiling is `MediaPrep.sizeLimit = 100 * 1024 * 1024` (MediaPrep.swift). Reachable from four call sites on real user taps: ConversationView.swift:1558 (`shareAttachment`), :1829 (`openFile` → Quick Look), MacConversationView.swift:1554, MacAttachmentViewer.swift:159 (Save) and :176 (Share). Note the atomic write briefly costs the Data plus a full on-disk copy.

> **Fix.** Add a streaming download to APIClient (`session.download(for:)` returning the temp URL) and have `localFileURL(for:)` `moveItem` it into the cache directory instead of round-tripping through `Data`. Keep the existing in-memory path only for previews.

**F120 — If a CallKit transaction is refused, the in-app End button does nothing and the call hangs until a 90-second guard**  
*ios · should-fix*

> **Evidence.** ios/FamilyConnect/Core/Calls/CallKitController.swift:144-149 — `private func request(_ action: CXAction) { callController.request(CXTransaction(action: action)) { error in if let error { AppLog.push.error("CallKit transaction failed: ...") } } }`. The error is logged and discarded; no caller is told. ios/FamilyConnect/Core/Calls/CallManager.swift:363-369 `hangUp()` routes exclusively through it: `if let systemBridge, let callUUID { systemBridge.requestEnd(callID: callUUID) } else { performHangUp(reportToSystem: false) }` — the local path runs only when there is no bridge at all, which on iOS there always is. The same applies to `routeAccept()` (:339-345) and to `reportOutgoing` (:57-62), so a refused `CXStartCallAction` means CallKit never knows the UUID and the later `CXEndCallAction` is refused too. The only thing that eventually clears it is `startGuard(seconds: ringTimeout * 2, ...)` = 90 s (CallManager.swift:625).

> **Fix.** Give `request(_:)` a completion (or a `onTransactionFailed` closure on `CallSystemBridge`) and, on error, fall back to `performHangUp(reportToSystem: false)` / `performAccept()` so the app's own state machine still moves when the system refuses.

**F121 — The PushKit handler runs a synchronous SwiftData fetch before reporting the incoming call to CallKit**  
*ios · nice-to-have*

> **Evidence.** ios/FamilyConnect/Core/Calls/VoIPPushRegistrar.swift:93 `let peer = callManager?.resolvePeer(push.fromUserID)` executes before :96 `callKit.reportIncoming(callID: uuid, ...) { _ in completion() }`. That closure is wired in ios/FamilyConnect/FamilyConnectApp.swift:137-143 to a blocking main-context query: `let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID }); if let member = try? container.mainContext.fetch(descriptor).first { ... }`. The file's own header (VoIPPushRegistrar.swift:8-11) states the rule being risked: "the app MUST report a call to CallKit before the completion handler runs, or iOS terminates it. Everything here is arranged around that sentence." The push payload already carries the name — `IncomingCallPush.callerName`, parsed at CallManager.swift:118-131.

> **Fix.** Report the call to CallKit first using `push.callerName` from the payload, call `completion()` from `reportNewIncomingCall`'s completion block, and only then resolve the member from the store and refine the display name with `provider.reportCall(with:updated:)` on a CXCallUpdate.

### `test-coverage` (9)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F124 | The macOS unit-test run never executes: the test host wedges forever in ModelContainer at launch | macos | nice-to-have |
| F125 | No CI, and 249 server integration tests are all #[ignore]d — the entire handler layer's coverage is opt-in | server | should-fix |
| F126 | THE most important missing test: nothing asserts the macOS app launches | macos | nice-to-have |
| F127 | `-destination platform=macOS` drags the iOS-only UI-test target into an iphoneos device build that needs a provisioning profile | macos | nice-to-have |
| F128 | ServerURLNormalizer — the first-run gate of a self-hosted app — has zero tests | both | nice-to-have |
| F129 | The Share Extension's half of the hand-off contract is untested and re-declares all four shared constants privately | both | nice-to-have |
| F130 | Voice messages: the recorder has no tests at all | both | nice-to-have |
| F131 | Location sharing: LocationProvider has no tests | both | nice-to-have |
| F132 | 3 of 4 UI tests contribute zero signal in CI — two XCTSkip without a live seeded server, one also needs a signed build | ios | nice-to-have |

**F124 — The macOS unit-test run never executes: the test host wedges forever in ModelContainer at launch**  
*macos · nice-to-have*

> **Evidence.** Reproduced twice. `xcodebuild test -destination 'platform=macOS,arch=arm64' -only-testing:FamilyConnectTests -skip-testing:FamilyConnectUITests` builds fine, launches the host, then hangs with 0 tests reported (run 2: host pid 26415 at 20:16, still 0 passed at 20:19; run 3: host pid 27056 launched 20:20:44, same stack). `sample` of the host, all 2218 samples in one stack: `FamilyConnectApp.init() ... FamilyConnectApp.swift:86 -> ModelContainer -> NSPersistentContainer loadPersistentStores -> NSSQLiteConnection connect -> openDatabase -> robust_open2 -> __guarded_open_np` (never returns; process at 0.11s CPU). ios/FamilyConnect/FamilyConnectApp.swift:85-86 is `let result = Result { try ModelContainer(for: schema, configurations: [configuration]) }`. `lsof -p 1175` (the installed /Applications/FamilyConnect.app, running since 19:09) holds `~/Library/Group Containers/group.me.nettrash.FamilyConnect/Library/Application Support/default.store` plus -wal/-shm on fds 3u/4u/5u; `lsof` on the test host shows no fd for that path. The same suite is 617/617 green in 26.4s on iOS Simulator.

> **Fix.** First determine whether it reproduces with no other instance running (quit /Applications/FamilyConnect.app, re-run). If it only happens with two instances, fix the test lane (kill any running copy in a pre-action, or give the test host its own container) and file the product risk. If it reproduces single-instance, treat it as a launch blocker and add a bounded/retrying store open with a visible failure state instead of an unguarded `try ModelContainer` on the main thread at FamilyConnectApp.swift:86.

**F125 — No CI, and 249 server integration tests are all #[ignore]d — the entire handler layer's coverage is opt-in**  
*server · should-fix*

> **Evidence.** `.github/workflows` does not exist (`ls .github` -> only `modernize`). server/tests/ holds 249 tests across 20 files, and every one is ignored: account_flow.rs 20/20, attachment_flow.rs 40/40, push_flow.rs 25/25, call_flow.rs 21/21, auth_flow.rs 8/8, etc., all with `#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]` (auth_flow.rs:10). They are the ONLY coverage for six modules: src/handlers_attachment.rs, src/handlers_board.rs, src/handlers_device.rs, src/handlers_poll.rs, src/handlers_stats.rs and src/storage.rs all have 0 `#[test]`/`#[tokio::test]`. On this machine they cannot run right now: `pg_isready -h 127.0.0.1` -> "no response", `docker ps` -> docker not running (the fc-test-pg container is down).

> **Fix.** Add one GitHub Actions workflow. Minimum worth having, in this order: (1) `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test --lib` — free, 0.72s; (2) `cargo test -- --ignored` with `services: postgres:16` and PGHOST/PGUSER/PGPASSWORD set — this is the single biggest win, it turns 249 dormant tests into a gate for zero extra test-writing; (3) `xcodebuild test -only-testing:FamilyConnectTests -destination 'platform=iOS Simulator,name=iPhone 17' CODE_SIGNING_ALLOWED=NO` on a macos runner, 26s. Skip the UI-test and macOS lanes for now (see the other findings). Its absence is not a release blocker — the code is written and green — but it should land before the second release, not after.

**F126 — THE most important missing test: nothing asserts the macOS app launches**  
*macos · nice-to-have*

> **Evidence.** FamilyConnectUITests SUPPORTED_PLATFORMS = "iphoneos iphonesimulator" (ios/FamilyConnect.xcodeproj/project.pbxproj:695, 718, 880) while the app and FamilyConnectTests carry "iphoneos iphonesimulator macosx" (:648, :672, :857). The only launch test that exists, ios/FamilyConnectUITests/FamilyConnectUITestsLaunchTests.swift:17 `testLaunch`, therefore cannot run on macOS. ios/FamilyConnect/MacViews/ is 4560 lines across 10 files (MacChatView, MacConversationView, MacBoardView, MacAttachmentViewer, MacSettingsView, ...) and no test file references any `Mac*View` symbol.

> **Fix.** Add `macosx` to FamilyConnectUITests SUPPORTED_PLATFORMS and add MacLaunchUITests.testMacAppLaunchesToServerSetup: `XCUIApplication()` with `launchArguments = ["--uitest-reset"]`, launch, assert the server-setup field exists within ~20s. No server, no fixture, no signing beyond the local profile — it is a dozen lines and it is the only thing standing between a broken Mac launch and the App Store.

**F127 — `-destination platform=macOS` drags the iOS-only UI-test target into an iphoneos device build that needs a provisioning profile**  
*macos · nice-to-have*

> **Evidence.** The FamilyConnect scheme's TestAction lists both FamilyConnectTests and FamilyConnectUITests (FamilyConnect.xcscheme, two TestableReference entries, both `skipped = "NO"`). Running `xcodebuild test -destination 'platform=macOS,arch=arm64' -only-testing:FamilyConnectTests` still built the UI target, and since it cannot build for macosx Xcode fell back to iphoneos: `ProcessXCFramework ... WebRTC.xcframework ... Debug-iphoneos ... --platform ios` (mac-test.log:72) and `CodeSign .../Debug-iphoneos/FamilyConnectUITests-Runner.app` with `Provisioning Profile: "iOS Team Provisioning Profile: me.nettrash.FamilyConnect.ShareExtension"` (mac-test.log:491). 351s elapsed before I interrupted it.

> **Fix.** Either add `macosx` to FamilyConnectUITests SUPPORTED_PLATFORMS (preferred — see no-macos-launch-smoke-test), or split a macOS-only shared scheme whose TestAction contains just FamilyConnectTests. Document `-skip-testing:FamilyConnectUITests` in README.md's test section as the interim command.

**F128 — ServerURLNormalizer — the first-run gate of a self-hosted app — has zero tests**  
*both · nice-to-have*

> **Evidence.** `grep -r ServerURLNormalizer ios/FamilyConnectTests ios/FamilyConnectUITests` returns 0 hits. ios/FamilyConnect/Core/ServerURLNormalizer.swift is 65 lines of pure, trivially testable parsing: implicit `https://` prefixing, trailing-slash stripping, the three-way Verdict (.ok / .okInsecureLocal / .invalid), and `isLocalNetworkHost` hand-rolling the RFC1918 / loopback / 169.254 / *.local / localhost table that must agree with the ATS NSAllowsLocalNetworking exception. Three consumers: Storage/AppSettings.swift:76, Views/ServerSetupView.swift:35 and :106.

> **Fix.** Add ServerURLNormalizerTests: a parameterized table over 172.15/172.16/172.31/172.32, 10.x, 192.168.x, 127.x, 169.254.x, localhost, foo.local, bare host -> https, trailing slashes, `ftp://`, empty, and http-to-public -> .invalid. Roughly 20 lines, and it also pins the ATS contract against Info.plist.

**F129 — The Share Extension's half of the hand-off contract is untested and re-declares all four shared constants privately**  
*both · nice-to-have*

> **Evidence.** ios/FamilyConnect/Core/ShareImport.swift:29-34 declares `scheme = "familyconnect"`, `host = "share"`, `appGroup = "group.me.nettrash.FamilyConnect"`, `inboxFolder = "ShareInbox"` with the comment "One spelling, here." ios/FamilyConnectShareExtension/ShareViewController.swift:43-46 declares its own private `appGroupIdentifier`, `handoffScheme`, `handoffHost`, `inboxFolder` with the same four literals. ShareImportTests.swift (10 tests) covers only the app's parsing side (`ShareImport.ids(from:)`, `inboxDirectory`, `isEligible`); no test file references `ShareViewController`, and the extension target has no test target, so `safeFileName` (:175), `bestFileName` (:163) and `handoffURL` (:196) are entirely uncovered.

> **Fix.** Make ShareViewController's four constants read from ShareImport (add the file to the extension target — it is `nonisolated enum` with no app dependencies), and add a test that round-trips producer to consumer: build the hand-off URL from N UUIDs and assert `ShareImport.ids(from:)` returns them. Add a small table for `safeFileName` (slashes, colons, leading dots, empty, >255 chars).

**F130 — Voice messages: the recorder has no tests at all**  
*both · nice-to-have*

> **Evidence.** `grep -rl AudioRecorder ios/FamilyConnectTests ios/FamilyConnectUITests` -> 0 files. ios/FamilyConnect/Core/AudioRecorder.swift is 173 lines; ios/FamilyConnect/Views/AudioPlayerView.swift is 153 lines and is likewise referenced by no test. What IS covered is only the resulting attachment DTO: AttachmentTests.swift:131 asserts a `duration_ms` query item, AttachmentTests.swift:879 the "2 Audio" summary, MediaOnlyTests.swift:78 that an audio row keeps its balloon.

> **Fix.** Extract the recorder's state machine and duration accounting behind a protocol the way CallManagerTests already does for the media client (CallManagerTests.swift:145 `audioSessionDidActivate`), and test start/stop/cancel/denied-permission over a fake sink. The AVFoundation call itself can stay untested.

**F131 — Location sharing: LocationProvider has no tests**  
*both · nice-to-have*

> **Evidence.** `grep -rl LocationProvider ios/FamilyConnectTests ios/FamilyConnectUITests` -> 0 files; ios/FamilyConnect/Core/LocationProvider.swift is 186 lines. Coverage of "location" is entirely on the receiving/rendering side: AttachmentTests.swift:365 decodes an inbound location DTO, UnreadRulesTests.swift:502 asserts a notification never leaks coordinates, MediaOnlyTests.swift:68 and AttachmentAlbumTests.swift:52 cover bubble layout.

> **Fix.** Add LocationProviderTests over a CLLocationManager seam: authorization denied yields a typed error rather than a hang, a fix older than N seconds is rejected, accuracy is carried through to `accuracy_m`. Mirrors the existing AvatarFailureTests pattern of one sentence per failure mode.

**F132 — 3 of 4 UI tests contribute zero signal in CI — two XCTSkip without a live seeded server, one also needs a signed build**  
*ios · nice-to-have*

> **Evidence.** ios/FamilyConnectUITests/ConversationScrollUITests.swift:28 and AttachmentViewerSwipeUITests.swift:38 both `throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")` when `FC_UITEST_SERVER` is unset. The fixtures come from server/scripts/seed-scroll-uitest.sh and seed-album-uitest.sh, which require a running `fc-test-pg` docker container plus `cargo run` on 127.0.0.1:8091. AttachmentViewerSwipeUITests.swift:19-21 adds: "Build SIGNED (no CODE_SIGNING_ALLOWED=NO): the app-group entitlement makes the Keychain refuse an unsigned build's token write, and the login step then fails." Only FamilyConnectUITests.swift:19 and FamilyConnectUITestsLaunchTests.swift:17 are hermetic.

> **Fix.** In CI, run only `-only-testing:FamilyConnectUITests/FamilyConnectUITests -only-testing:FamilyConnectUITests/FamilyConnectUITestsLaunchTests`. Separately, make the skip loud: `XCTFail` instead of `XCTSkip` when an env var like `FC_UITEST_REQUIRED=1` is set, so a nightly seeded lane cannot pass by skipping. Add the seed-plus-run recipe to README.md's test section, where it is currently absent.

### `competitive-review-lens` (12)

| # | Finding | Platform | Severity |
|---|---|---|---|
| F136 | No working demo account: review notes ship literal [DEMO_USER]/[DEMO_PASS] placeholders, and one account can exercise almost nothing | both | should-fix |
| F137 | The description and review notes describe a text-only app; the binary ships media, voice/video calls, location, polls, a share extension and an AI assistant | both | should-fix |
| F138 | No privacy policy URL, no support URL, and no privacy-policy link inside the app | both | blocker |
| F139 | A messaging app with open registration on the developer's server, and no Report, no Block, and no content filtering anywhere | both | should-fix |
| F140 | PrivacyInfo.xcprivacy declares NSPrivacyCollectedDataTypes as an empty array for an app that uploads messages, photos, voice, files and precise location to a developer-run server | both | should-fix |
| F141 | Chat text goes to Azure OpenAI and every call leaks device IPs to Google's public STUN server, while the listing claims no third parties | server | should-fix |
| F142 | macOS: saving a received photo/file via the Save panel is denied by the sandbox and the error is swallowed — nothing happens | macos | should-fix |
| F143 | appstore.md is iOS-only, yet the same target ships a Mac app and declares iPad support — no Mac/iPad screenshots, notes or category exist | both | should-fix |
| F144 | Calls have no relay: with turn_urls empty by default, a reviewer's call rings for 45 s and ends — they will conclude the feature is broken | server | nice-to-have |
| F145 | The review notes still open with a BLOCKER saying account deletion does not exist — it does | process | nice-to-have |
| F146 | ITSAppUsesNonExemptEncryption is not in the built Info.plist — every upload stalls on the export-compliance question | both | nice-to-have |
| F147 | No age-rating answers in the metadata for an app with unmoderated UGC and an in-app AI chatbot | process | nice-to-have |

**F136 — No working demo account: review notes ship literal [DEMO_USER]/[DEMO_PASS] placeholders, and one account can exercise almost nothing**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md, "Notes for App Review": "Owner: username [DEMO_USER], password [DEMO_PASS]" / "Second account: username [DEMO_USER_2], password [DEMO_PASS_2]" / "join the reviewer family with invite code [INVITE_CODE]". Every box in the "Pre-submission checklist" is unchecked, including "Create the two demo accounts on fc.nettrash.me" and "Enter the owner demo credentials in App Review Information → demo account fields too". The cold-launch path is ServerSetupView → AuthView (a sign-in wall: ios/FamilyConnect/Views/AuthView.swift), and a lone self-registered account lands on FamilyGateView → creates a family → is alone: ios/FamilyConnect/Views/NewChatView.swift:38-41 shows ContentUnavailableView("No one else yet", … "Direct chats become available once another member joins the family.").

> **Fix.** Provision both demo accounts and the reviewer family on fc.nettrash.me, seed family + 1:1 message history and a couple of media messages, replace all five bracketed placeholders in ios/docs/appstore.md, and enter the owner credentials in App Review Information → Sign-in required → demo account. Keep the second account signed in on a device (or add a note that the reviewer should sign in on two devices) so calls, receipts and typing can actually be seen.

**F137 — The description and review notes describe a text-only app; the binary ships media, voice/video calls, location, polls, a share extension and an AI assistant**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md Description: "the honest limits of version 1: text messages only. Voice and video calls are planned." and "What it does not have: ads, analytics, tracking, or third-party SDKs of any kind." Review notes repeat: "Text messages only in this version." The binary contradicts all three: ios/FamilyConnect/Views/CallView.swift + Core/Calls/CallManager.swift + CallKitController.swift (WebRTC calls with CallKit), Views/AttachmentView.swift / AttachmentAlbum.swift / AudioPlayerView.swift / LocationAttachmentView.swift / PollComposerView.swift / BoardView.swift, FamilyConnectShareExtension/, and one SPM binary dependency (stasel/WebRTC 151.0.0) that is by definition a third-party SDK — its framework is embedded in the built product (…/Release-nettrash/FamilyConnect.app/Frameworks, WebRTC.framework). Info.plist declares UIBackgroundModes audio+voip and INIntentsSupported INStartCallIntent; INFOPLIST_KEY_NSCameraUsageDescription in project.pbxproj:546 begins "Video calls show you to your family…".

> **Fix.** Rewrite the Description, the Beta App Description and the "Notes for App Review" in ios/docs/appstore.md against the shipped feature set (photos/videos/albums, voice messages, files, location sharing, polls, boards, reactions, edits, share extension, WebRTC voice+video with CallKit, optional AI assistant), delete the "no third-party SDKs of any kind" claim (WebRTC is one), and list each permission the app asks for and why.

**F138 — No privacy policy URL, no support URL, and no privacy-policy link inside the app**  
*both · blocker*

> **Evidence.** /Users/nettrash/Develop/nettrash.me/nettrash-me/frontend/assets/appstore/ contains only exchange, geo, md, scan — nothing for Family Connect; grep for "family"/"familyconnect" in that repo's src returns only unrelated component files. ios/docs/appstore.md's checklist still has "[ ] Fill [SUPPORT_EMAIL]; set Support URL and Privacy Policy URL" unchecked, and the review notes end with "contact [SUPPORT_EMAIL]". Inside the app: `grep -rniE "terms of use|EULA|privacy policy|support@" ios/FamilyConnect/**/*.swift` returns ZERO hits. SettingsView.swift:312-325 has a section literally titled "Privacy", but it is only a link-preview/map toggle — no policy, no contact, no terms.

> **Fix.** Add /appstore/familyconnect/privacy.html and /appstore/familyconnect/support.html to the nettrash-me site covering both modes (developer-operated default server vs. self-hosted), fill the Support URL / Privacy Policy URL / support e-mail fields, and add an "About" section in SettingsView with tappable Privacy Policy, Terms and Support links (plus the same in MacSettingsView).

**F139 — A messaging app with open registration on the developer's server, and no Report, no Block, and no content filtering anywhere**  
*both · should-fix*

> **Evidence.** `grep -rniE "report|blockUser|abuse|moderat|objectionable"` over ios/FamilyConnect returns no user-facing report/block feature (only unrelated words like reportOutgoing/read report), and the server has no such route either. The only moderation control is owner-only: ios/FamilyConnect/Views/FamilyManageView.swift:286 `Button(role: .destructive) { remove(member) }` inside the block commented "Remove and Password are owner actions" — a non-owner member being harassed in a 1:1 chat has no block, no mute, no report. Registration on the shipped default server is completely ungated: server/src/handlers_auth.rs:115 `pub async fn register(...)` validates username/display name/password and INSERTs — there is no invite gate, no allow-list, and no `[registration]` switch in server/src/config.rs. ios/docs/appstore.md's own checklist admits it: "Decide whether open registration on fc.nettrash.me is acceptable long-term — any App Store user can register and create their own family on your box". The app also has an in-app AI assistant that generates text into a chat (ios/FamilyConnect/Views/FamilyAssistantSettings.swift; strings "Ask the assistant", "Assistant").

> **Fix.** Add per-user Block (client-side hide plus a server-side mute so a blocked member's messages and calls do not arrive) and Report on a message and on a member, with an in-app support/contact address; state the moderation response commitment in the review notes. Alternatively, close registration on fc.nettrash.me behind an invite so the default server genuinely cannot host strangers — and say so in the notes — but Apple will still expect block/report inside a messaging app.

**F140 — PrivacyInfo.xcprivacy declares NSPrivacyCollectedDataTypes as an empty array for an app that uploads messages, photos, voice, files and precise location to a developer-run server**  
*both · should-fix*

> **Evidence.** ios/FamilyConnect/PrivacyInfo.xcprivacy: `<key>NSPrivacyCollectedDataTypes</key><array/>` — empty — with only a CA92.1 UserDefaults reason declared. The app plainly collects: message text and attachments (server/src/handlers_* ; Views/AttachmentView.swift), photos and videos, voice messages (Core/AudioRecorder.swift), files (share extension), precise location (Core/LocationProvider.swift:54 CLLocationManager, INFOPLIST_KEY_NSLocationWhenInUseUsageDescription), a username and display name, and a birthday (Views/BirthdayView.swift). ios/docs/appstore.md's own checklist says "because the store build defaults to the developer-operated server, 'Data Not Collected' is NOT defensible any more" — yet the shipped manifest says exactly that. With the assistant enabled the same content leaves the server entirely: server/src/ai.rs:1 "//! The assistant: Azure OpenAI chat completions, streamed."

> **Fix.** Declare the real collected types in PrivacyInfo.xcprivacy (user content, user ID, coarse/precise location, contact info if the birthday counts), each with linked/tracking = false and App Functionality purposes, and fill the App Privacy label to match — including a third-party-sharing answer for the Azure OpenAI assistant when it is enabled on the default server.

**F141 — Chat text goes to Azure OpenAI and every call leaks device IPs to Google's public STUN server, while the listing claims no third parties**  
*server · should-fix*

> **Evidence.** server/src/ai.rs:1-9: "The assistant: Azure OpenAI chat completions, streamed… A request carries the configured system prompt and the last N messages of ONE member's OWN assistant chat"; the client exposes it ("Ask the assistant", and the owner setting whose string reads "With this on, mentioning %@ in the family chat sends the last month of that chat to the assistant"). server/src/config.rs:987-989: `fn default_stun_urls() -> Vec<String> { vec!["stun:stun.l.google.com:19302".to_string()] }` — the shipped default sends a STUN binding request, and therefore the device's IP, to Google on every call. ios/docs/appstore.md claims "no third-party SDKs of any kind" and "Message and account data exist only on the server the user chose and on the devices."

> **Fix.** Either turn the assistant off on the default server and say so, or disclose it: name Azure OpenAI in the description, the privacy policy and the App Privacy third-party-sharing answer, and add an explicit in-app consent before a member's first assistant message. Point stun_urls at a self-hosted STUN/TURN (coturn) instead of Google, or disclose the STUN dependency.

**F142 — macOS: saving a received photo/file via the Save panel is denied by the sandbox and the error is swallowed — nothing happens**  
*macos · should-fix*

> **Evidence.** ios/FamilyConnect/MacViews/MacAttachmentViewer.swift:154-168 runs an NSSavePanel and then `try? FileManager.default.removeItem(at: destination)` / `try? FileManager.default.copyItem(at: source, to: destination)` — both errors discarded. ios/FamilyConnect/FamilyConnect-macOS.entitlements grants only `com.apple.security.files.user-selected.read-only`; there is no `com.apple.security.files.user-selected.read-write`, which is what a sandboxed app needs to WRITE to a location chosen in a Save panel. Net effect: the panel appears, the user picks a folder, and no file is written and no error is shown.

> **Fix.** Add `com.apple.security.files.user-selected.read-write` to FamilyConnect-macOS.entitlements, and replace the two `try?` with real error handling that surfaces a failure to the user.

**F143 — appstore.md is iOS-only, yet the same target ships a Mac app and declares iPad support — no Mac/iPad screenshots, notes or category exist**  
*both · should-fix*

> **Evidence.** ios/docs/appstore.md never mentions macOS or iPad anywhere. The project builds and ships both: `xcodebuild -scheme FamilyConnect-nettrash -configuration Release-nettrash -destination platform=macOS` → BUILD SUCCEEDED, producing …/Products/Release-nettrash/FamilyConnect.app with FCDefaultServerURL = https://fc.nettrash.me and LSMinimumSystemVersion 14.0; project.pbxproj sets TARGETED_DEVICE_FAMILY = "1,2" and INFOPLIST_KEY_UISupportedInterfaceOrientations_iPad with all four orientations. No screenshots exist anywhere in ios/ (the only screenshot directory in the repo is android/fastlane/metadata/android/en-US/images/phoneScreenshots). The iPad runs the iPhone layout: NavigationSplitView appears only in MacViews/MacChatView.swift:51 — nothing in ios/FamilyConnect/Views/ adapts to a regular size class.

> **Fix.** Either drop iPad from TARGETED_DEVICE_FAMILY for 1.0, or give the iPad a NavigationSplitView layout and capture iPad screenshots. Add a macOS section to ios/docs/appstore.md (its own description, screenshots, category and review notes, and a statement about universal purchase), and produce iPhone/iPad/Mac screenshots for the localizations you ship.

**F144 — Calls have no relay: with turn_urls empty by default, a reviewer's call rings for 45 s and ends — they will conclude the feature is broken**  
*server · nice-to-have*

> **Evidence.** server/src/config.rs:79-82: "TURN servers… Empty means no relay: calls that cannot connect directly simply fail", and the default is `turn_urls: Vec::new()` (config.rs:120) with only public STUN. ios/FamilyConnect/Core/Calls/CallManager.swift:261-262 `/// The server's ring timeout (45 s); var ringTimeout: TimeInterval = 45`. On iOS the direct path on a shared Wi-Fi also depends on the Local Network permission (INFOPLIST_KEY_NSLocalNetworkUsageDescription in project.pbxproj:547) — declined or unprompted, same-LAN candidates are silently dropped.

> **Fix.** Stand up coturn for fc.nettrash.me and set turn_urls/turn_secret before submitting; in the review notes, tell the reviewer exactly how to place a call (which two accounts, which devices, allow the Local Network prompt), and show a specific failure message in CallView when ICE fails rather than a generic ended state.

**F145 — The review notes still open with a BLOCKER saying account deletion does not exist — it does**  
*process · nice-to-have*

> **Evidence.** ios/docs/appstore.md, top of file: "> **BLOCKER before submitting** (guideline 5.1.1(v)): the app supports account creation, so Apple requires an in-app account deletion flow. v1 does not have one yet…" and the checklist's first unchecked item repeats it. The feature is in fact implemented: ios/FamilyConnect/Views/DeleteAccountView.swift, ios/FamilyConnect/Views/SettingsView.swift:350 `Button("Delete Account", role: .destructive)`, MacViews/MacSettingsView.swift, server route POST /api/v1/me/delete (server/src/handlers_auth.rs delete_account), and ios/FamilyConnectTests/AccountDeletionTests.swift.

> **Fix.** Delete the stale blocker blockquote and the corresponding checklist line, and keep only the accurate ACCOUNT DELETION paragraph that already tells the reviewer the exact path (Settings → Account → Delete Account).

**F146 — ITSAppUsesNonExemptEncryption is not in the built Info.plist — every upload stalls on the export-compliance question**  
*both · nice-to-have*

> **Evidence.** `PlistBuddy -c Print` on the built store bundle (…/Release-nettrash-iphonesimulator/FamilyConnect.app/Info.plist) shows no ITSAppUsesNonExemptEncryption key; the same is true of the macOS Release-nettrash bundle. The app does use encryption: HTTPS to the server, SHA-256 session tokens, and WebRTC/SRTP for calls.

> **Fix.** Add `ITSAppUsesNonExemptEncryption = NO` (with `ITSEncryptionExportComplianceCode` if a code is issued) to both ios/FamilyConnect/Info.plist and Info-macOS.plist, after confirming the app only uses exempt encryption (HTTPS/standard OS crypto plus WebRTC's SRTP).

**F147 — No age-rating answers in the metadata for an app with unmoderated UGC and an in-app AI chatbot**  
*process · nice-to-have*

> **Evidence.** ios/docs/appstore.md has no Age Rating section at all, while LSApplicationCategoryType is public.app-category.social-networking (project.pbxproj:545) and the app carries free-form user-generated text, photos, video and files between accounts that anyone can create (server/src/handlers_auth.rs:115), plus an assistant that generates text into a chat (server/src/ai.rs; strings "Ask the assistant", "Assistant language").

> **Fix.** Add an Age Rating section to ios/docs/appstore.md recording the answers actually given (UGC: yes; in-app AI chatbot: yes when the assistant is enabled) and the resulting rating, and keep it in step with whatever moderation controls you add for the 1.2 finding.

---
## Appendix B — refuted (3)
Claimed by a finder, killed by verification. Recorded so they are not re-raised.

- ~~The 1024x1024 App Store icon source PNG carries an alpha channel~~
- ~~App Groups is not enabled on the macOS side of App ID me.nettrash.FamilyConnect — the Mac profile grants no group entitlement~~
- ~~macOS submission: hardened runtime is never enabled, and the share extension's App Group is not team-ID prefixed~~

---
<sub>Generated from the audit run `wf_6134d616-a32`. Regenerate by re-running the workflow, not by
hand-editing this file.</sub>
