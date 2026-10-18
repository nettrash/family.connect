//
//  RichWebHost.swift
//  FamilyConnect
//
//  The hidden web view that draws a rich block (issue #97; the design doc's
//  "The native host"). One per app, made on first need and kept.
//
//  WHAT IT LOADS. `rich.html` from the app's own copy of `web/rich/`,
//  over a real origin — the `fcrich:` scheme, answered by
//  `RichSchemeHandler` with the bundled files and nothing else. The page
//  pulls each engine in on first use (fc-rich.js `ensure`), from the same
//  origin.
//
//  WHAT IT REFUSES. Everything else, three times over: the page's own
//  content policy says `default-src 'none'`; a content rule list blocks
//  every load whose URL is not `fcrich:`; and the navigation delegate
//  cancels any navigation away from the page. Its data store is
//  non-persistent, so nothing a page could write outlives the process.
//  Mermaid runs with `securityLevel: 'strict'` and KaTeX with
//  `trust: false` (fc-rich.js), so a message cannot run script, fetch a
//  picture or link anywhere — and if one found a way, there is no network
//  for it to reach.
//
//  HOW A JOB RUNS. One at a time: `callAsyncJavaScript("return await
//  fcRich.draw(job)")` — Apple's call can await the page's promise, so no
//  message bridge is needed — then a snapshot of the rectangle the result
//  reports, at the page's top left. Fifteen seconds without an answer is
//  a failure, and the web view is thrown away with it: a formula that
//  hangs the engine hangs the page's one thread, and every job behind it
//  would time out too. The next job makes a fresh one. So does a web
//  content process that died (`webViewWebContentProcessDidTerminate`).
//
//  THE PICTURE keeps its alpha: the view is non-opaque with a clear
//  background (iOS), or does not draw its background (macOS), and a
//  formula is drawn on `transparent`. It is taken at the screen's scale,
//  capped so its longest side is at most 4096 pixels.
//
//  WHERE IT LIVES. On iOS a web view needs no window to lay out, run
//  script or be snapshotted (the md app's export does exactly this). On
//  macOS it does — WebKit only renders a view that is backed by a live
//  window — so the view is parked in a borderless, non-activating panel
//  far off screen, ordered behind everything: md.macOS's WebRenderer, and
//  the same trick. The panel cannot become key or main, is not in the
//  Window menu and ignores the mouse, so nothing that counts the app's
//  windows (MacMenuBar, LaunchWindowBackstop) ever sees it as one.
//
//  FONTS. KaTeX measures a formula the moment it sets it, and a face that
//  has not loaded yet is measured in a fallback font — a box the real
//  glyphs then overflow. So before a formula every KaTeX face that is not
//  loaded is (`document.fonts`, each `FontFace.load()`), and every job
//  waits for `document.fonts.ready` before its snapshot.
//

import Foundation
import os
import WebKit
#if os(macOS)
import AppKit
#else
import UIKit
#endif

@MainActor
final class RichWebHost: NSObject, WKNavigationDelegate {

    /// The scheme the page is served on.
    static let scheme = "fcrich"
    static let pageURL = URL(string: "fcrich://rich/rich.html")!

    /// Fifteen seconds without an answer is a failure (docs/protocol.md).
    static let timeout: Duration = .seconds(15)

    /// The view's size before a job asks for more. Diagrams are drawn at
    /// most 1200 CSS pixels wide (fc-rich.js's default `maxWidth`), so most
    /// fit without a resize.
    static let stageSize = CGSize(width: 1200, height: 1200)

    /// The longest side, in pixels, a drawing may have.
    static let maxPixels: CGFloat = 4096

    private var webView: WKWebView?
    #if os(macOS)
    private var panel: NSPanel?
    #endif
    /// Waiting for the page to finish loading.
    private var pageWaiters: [CheckedContinuation<Bool, Never>] = []
    private var pageLoaded = false
    /// Which web view a page wait belongs to, so a timeout set for one
    /// that was thrown away cannot fail the waiters of the next.
    private var pageGeneration = 0
    /// The call in flight, so a dying web content process can fail it.
    private var inFlight: OneShot<[String: Any]?>?
    private var queue: [(job: RichJob, done: CheckedContinuation<RichOutcome, Never>)] = []
    private var busy = false
    private var ruleList: WKContentRuleList?
    private var ruleListCompiled = false

    /// Draw one job — queued behind any other.
    func draw(_ job: RichJob) async -> RichOutcome {
        await withCheckedContinuation { continuation in
            queue.append((job, continuation))
            pump()
        }
    }

    private func pump() {
        guard !busy, !queue.isEmpty else { return }
        busy = true
        let next = queue.removeFirst()
        Task { @MainActor in
            let outcome = await self.run(next.job)
            next.done.resume(returning: outcome)
            self.busy = false
            self.pump()
        }
    }

    private func run(_ job: RichJob) async -> RichOutcome {
        guard let webView = await readyWebView() else {
            AppLog.ui.error("Rich renderer: the page did not load")
            return .failed
        }
        guard let result = await evaluate(job, in: webView) else {
            // Timed out, or the process died: start over next time.
            AppLog.ui.error("Rich renderer: a \(job.kind, privacy: .public) job got no answer")
            discard()
            return .failed
        }
        guard (result["ok"] as? Bool) == true,
              let width = (result["width"] as? NSNumber)?.doubleValue,
              let height = (result["height"] as? NSNumber)?.doubleValue,
              width > 0, height > 0
        else {
            // The engine's own message quotes the source: never logged.
            AppLog.ui.info("Rich renderer: a \(job.kind, privacy: .public) block could not be drawn")
            return .failed
        }
        let baseline = (result["baseline"] as? NSNumber)?.doubleValue ?? height
        guard let image = await snapshot(width: width, height: height, in: webView) else {
            AppLog.ui.error("Rich renderer: a \(job.kind, privacy: .public) snapshot failed")
            return .failed
        }
        return .drawn(RichDrawing(image: image, width: width, height: height, baseline: baseline))
    }

    // MARK: - The web view

    /// The web view with its page loaded, made if there is none.
    private func readyWebView() async -> WKWebView? {
        if let webView, pageLoaded { return webView }
        if webView == nil {
            await compileRuleList()
            make()
        }
        guard webView != nil else { return nil }
        let generation = pageGeneration
        let loaded = await withCheckedContinuation { continuation in
            pageWaiters.append(continuation)
            // A page that never finishes loading is a failure too.
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: Self.timeout)
                guard let self, self.pageGeneration == generation, !self.pageLoaded else { return }
                self.discard()
            }
        }
        return loaded ? webView : nil
    }

    private func make() {
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(RichSchemeHandler(), forURLScheme: Self.scheme)
        configuration.websiteDataStore = .nonPersistent()
        configuration.suppressesIncrementalRendering = true
        if let ruleList {
            configuration.userContentController.add(ruleList)
        }
        let view = WKWebView(frame: CGRect(origin: .zero, size: Self.stageSize), configuration: configuration)
        view.navigationDelegate = self
        #if os(macOS)
        // No background of its own, so a formula's pixels around the ink
        // stay transparent.
        view.setValue(false, forKey: "drawsBackground")
        view.underPageBackgroundColor = .clear
        let panel = NSPanel(
            contentRect: NSRect(
                x: -Self.stageSize.width - 10_000, y: -Self.stageSize.height - 10_000,
                width: Self.stageSize.width, height: Self.stageSize.height),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.isReleasedWhenClosed = false
        panel.isExcludedFromWindowsMenu = true
        panel.ignoresMouseEvents = true
        panel.hidesOnDeactivate = false
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.collectionBehavior = [.transient, .ignoresCycle, .stationary]
        panel.contentView = view
        panel.orderBack(nil)
        self.panel = panel
        #else
        view.isOpaque = false
        view.backgroundColor = .clear
        view.scrollView.backgroundColor = .clear
        view.scrollView.isScrollEnabled = false
        view.scrollView.contentInsetAdjustmentBehavior = .never
        #endif
        webView = view
        pageLoaded = false
        pageGeneration &+= 1
        view.load(URLRequest(url: Self.pageURL))
    }

    /// Throw the web view away; the next job makes a new one.
    private func discard() {
        webView?.navigationDelegate = nil
        webView?.stopLoading()
        #if os(macOS)
        panel?.orderOut(nil)
        panel?.contentView = nil
        panel = nil
        #endif
        webView = nil
        pageLoaded = false
        finishLoading(false)
    }

    private func finishLoading(_ loaded: Bool) {
        if loaded { pageLoaded = true }
        let waiters = pageWaiters
        pageWaiters = []
        for waiter in waiters { waiter.resume(returning: loaded) }
    }

    /// Block every load that is not one of our files — the third wall (see
    /// the file header). Compiled once; a failure to compile leaves the
    /// other two.
    private func compileRuleList() async {
        guard !ruleListCompiled else { return }
        ruleListCompiled = true
        let rules = """
            [
              {"trigger": {"url-filter": ".*"}, "action": {"type": "block"}},
              {"trigger": {"url-filter": "^fcrich:"}, "action": {"type": "ignore-previous-rules"}}
            ]
            """
        do {
            ruleList = try await WKContentRuleListStore.default()?.compileContentRuleList(
                forIdentifier: "FamilyConnectRichOnly", encodedContentRuleList: rules)
        } catch {
            AppLog.ui.error("Rich renderer: the content rules did not compile")
        }
    }

    // MARK: - A job

    /// A formula's call: every KaTeX face loaded FIRST (see the file
    /// header), then the draw, then `fonts.ready` so nothing is snapshotted
    /// mid-load.
    ///
    /// Before EVERY formula, not once: the `FontFace` objects a stylesheet
    /// declares are rebuilt whenever the page's styles change — Mermaid
    /// adds its own — and come back `unloaded` (measured: all twenty
    /// loaded after the first formula, none after a diagram). Their bytes
    /// are cached by then, so a second load costs a promise each.
    ///
    /// And if the draw itself still had to load a face — Mermaid's styles
    /// can land a moment late, rebuilding the faces between the loads and
    /// the draw (seen under heavy load) — the formula was measured in a
    /// fallback font, so it is drawn once more with every face it uses in
    /// place.
    private static let formulaScript = """
        const loaded = () => Array.from(document.fonts).filter(f => f.status === 'loaded').length;
        if (await fcRich.prepare('math')) {
          await Promise.all(Array.from(document.fonts)
            .filter(f => f.status !== 'loaded')
            .map(f => f.load().catch(() => null)));
        }
        const before = loaded();
        let r = await fcRich.draw(job);
        await document.fonts.ready;
        if (r && r.ok && loaded() > before) {
          r = await fcRich.draw(job);
          await document.fonts.ready;
        }
        return r;
        """

    private static let diagramScript = """
        const r = await fcRich.draw(job);
        await document.fonts.ready;
        return r;
        """

    /// `fcRich.draw(job)`, or nil after fifteen seconds without an answer
    /// or when the web content process died under it.
    private func evaluate(_ job: RichJob, in webView: WKWebView) async -> [String: Any]? {
        let call = OneShot<[String: Any]?>()
        inFlight = call
        defer { if inFlight === call { inFlight = nil } }
        return await call.wait { finish in
            webView.callAsyncJavaScript(
                job.isFormula ? Self.formulaScript : Self.diagramScript,
                arguments: ["job": job.arguments], in: nil, in: .page
            ) { result in
                switch result {
                case .success(let value): finish(value as? [String: Any])
                case .failure: finish(nil)
                }
            }
            Task { @MainActor in
                try? await Task.sleep(for: Self.timeout)
                finish(nil)
            }
        }
    }

    /// How many of the page's font faces have loaded, and failed, of how
    /// many it declares — for the test that proves KaTeX's fonts reach the
    /// page.
    func fontFaces() async -> (loaded: Int, failed: Int, total: Int)? {
        guard let webView = await readyWebView() else { return nil }
        let script = """
            const faces = Array.from(document.fonts);
            return [faces.filter(f => f.status === 'loaded').length,
                    faces.filter(f => f.status === 'error').length, faces.length];
            """
        guard let value = try? await webView.callAsyncJavaScript(script, arguments: [:], in: nil, contentWorld: .page),
              let counts = value as? [NSNumber], counts.count == 3
        else { return nil }
        return (counts[0].intValue, counts[1].intValue, counts[2].intValue)
    }

    /// The rectangle at the page's top left, as pixels with alpha, at the
    /// screen's scale but never more than `maxPixels` on its longest side.
    private func snapshot(width: Double, height: Double, in webView: WKWebView) async -> CGImage? {
        // Big enough to hold it: a snapshot outside the view's bounds is
        // blank.
        let needed = CGSize(
            width: max(Self.stageSize.width, ceil(width)), height: max(Self.stageSize.height, ceil(height)))
        if webView.frame.size != needed {
            #if os(macOS)
            panel?.setContentSize(needed)
            #endif
            webView.frame = CGRect(origin: .zero, size: needed)
            // A beat for the web process to lay out and paint the new area.
            try? await Task.sleep(for: .milliseconds(100))
        }
        let rect = CGRect(x: 0, y: 0, width: width, height: height)
        // First at one point per point, which comes back at the screen's
        // own scale — and that scale is exactly what is wanted, unless it
        // would make the picture too big.
        guard let first = await takeSnapshot(rect, width: width, in: webView) else { return nil }
        let screenScale = CGFloat(first.width) / CGFloat(width)
        let longest = CGFloat(max(width, height))
        guard longest * screenScale > Self.maxPixels else { return first }
        let capped = Self.maxPixels / longest
        return await takeSnapshot(rect, width: width * Double(capped / screenScale), in: webView)
    }

    private func takeSnapshot(_ rect: CGRect, width: Double, in webView: WKWebView) async -> CGImage? {
        let configuration = WKSnapshotConfiguration()
        configuration.rect = rect
        configuration.snapshotWidth = NSNumber(value: width)
        configuration.afterScreenUpdates = true
        guard let image = try? await webView.takeSnapshot(configuration: configuration) else { return nil }
        #if os(macOS)
        // The bitmap itself, at its own pixel size: asking an NSImage for
        // "a CGImage" without a context gives one at ONE pixel per point.
        let bitmaps = image.representations.compactMap { $0 as? NSBitmapImageRep }
        if let best = bitmaps.max(by: { $0.pixelsWide < $1.pixelsWide }), let cgImage = best.cgImage {
            return cgImage
        }
        var proposed = CGRect(origin: .zero, size: image.size)
        return image.cgImage(forProposedRect: &proposed, context: nil, hints: nil)
        #else
        return image.cgImage
        #endif
    }

    // MARK: - WKNavigationDelegate

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard webView === self.webView else { return }
        finishLoading(true)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        guard webView === self.webView else { return }
        discard()
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
    ) {
        guard webView === self.webView else { return }
        discard()
    }

    /// The page itself and nothing else: no link, no redirect, no frame
    /// from anywhere but our own files.
    func webView(
        _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        decisionHandler(navigationAction.request.url?.scheme == Self.scheme ? .allow : .cancel)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        guard webView === self.webView else { return }
        AppLog.ui.error("Rich renderer: the web content process died")
        inFlight?.finish(nil)
        discard()
    }
}

/// A continuation that resumes ONCE, whichever of its finishers comes
/// first — the answer, the timeout or a dead process.
@MainActor
final class OneShot<Value> {
    private var continuation: CheckedContinuation<Value, Never>?
    private var finished = false

    func wait(_ start: (@escaping @MainActor @Sendable (Value) -> Void) -> Void) async -> Value {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
            start { [weak self] value in self?.finish(value) }
        }
    }

    func finish(_ value: Value) {
        guard !finished, let continuation else { return }
        finished = true
        self.continuation = nil
        continuation.resume(returning: value)
    }
}

/// Serves `fcrich://rich/…` from the bundled renderer files, and refuses
/// everything else (RichFiles holds the list).
@MainActor
final class RichSchemeHandler: NSObject, WKURLSchemeHandler {

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        guard let url = task.request.url, url.host == "rich",
              let file = RichFiles.url(forPath: url.path),
              let data = try? Data(contentsOf: file, options: .mappedIfSafe)
        else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        let response = HTTPURLResponse(
            url: url, statusCode: 200, httpVersion: "HTTP/1.1",
            headerFields: [
                "Content-Type": RichFiles.mimeType(for: file.lastPathComponent),
                "Content-Length": "\(data.count)",
            ])
        guard let response else {
            task.didFailWithError(URLError(.cannotParseResponse))
            return
        }
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {}
}
