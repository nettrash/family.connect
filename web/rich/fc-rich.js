/*
 * fc-rich.js — draws the rich blocks of a message (docs/protocol.md, "Rich blocks"):
 * formulas (KaTeX), Mermaid diagrams, Graphviz graphs and plot graphs.
 *
 * ONE renderer for every client. The web client calls `fcRich.renderInto` on an element in
 * the page; the native clients load `rich.html` in a hidden web view, call `fcRich.draw`
 * (or `fcRich.drawAndPost`), and snapshot the rectangle it reports into an image for the
 * bubble. Nothing here touches the network: every engine is a file next to this one, and
 * a host page must refuse every request that is not for one of them.
 *
 * A JOB is a plain object:
 *   kind        "math" (a display formula), "math-inline", "mermaid", "graphviz", "plot"
 *   source      the text between the fences or the dollar signs, exactly as written
 *   engine      graphviz only: dot, neato, circo, fdp, sfdp, twopi, osage or patchwork
 *   dark        true for the dark theme (Mermaid's theme; nothing else reads it)
 *   color       the ink, any CSS colour: the bubble's text colour for a formula, the
 *               card's text colour for a diagram or a graph
 *   background  "transparent" for a formula; the card colour for a diagram or a graph
 *   fontSize    CSS pixels; the bubble's body text size, which a formula is set at
 *   maxWidth    CSS pixels a diagram or a graph is scaled down to fit (default 1200)
 *   maxHeight   the same, for height (default 3000)
 *   mathml      web only: also emit MathML for screen readers (a snapshot must not,
 *               or the formula is drawn twice)
 *
 * A RESULT is {ok: true, width, height, baseline} — CSS pixels, the rectangle drawn at
 * the element's top left, and for a formula the distance from its top to its baseline
 * (the host lines an inline formula up with the words by it) — or {ok: false, error}
 * with the engine's own message. A block that draws nothing (an empty fence) is a
 * failure too, "empty": the host shows the source, as it does for any failure.
 *
 * The version below goes into every native cache key: change it with any file here.
 */
(function () {
  'use strict';

  var VERSION = '1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6';
  var GRAPHVIZ_ENGINES = ['dot', 'neato', 'circo', 'fdp', 'sfdp', 'twopi', 'osage', 'patchwork'];
  var CARD_PADDING = 12;

  // Where this file was loaded from, so the engines beside it load from the same place,
  // with the same query (the web client's cache-busting `?v=`).
  var here = (function () {
    var src = document.currentScript && document.currentScript.src;
    if (!src) return { base: '', query: '' };
    var q = src.indexOf('?');
    var path = q < 0 ? src : src.slice(0, q);
    return { base: path.slice(0, path.lastIndexOf('/') + 1), query: q < 0 ? '' : src.slice(q) };
  })();

  var loading = {};

  function load(file, isStyle) {
    if (loading[file]) return loading[file];
    loading[file] = new Promise(function (resolve, reject) {
      var el;
      if (isStyle) {
        el = document.createElement('link');
        el.rel = 'stylesheet';
        el.href = here.base + file + here.query;
      } else {
        el = document.createElement('script');
        el.src = here.base + file + here.query;
      }
      el.onload = function () { resolve(); };
      el.onerror = function () {
        delete loading[file];
        reject(new Error('could not load ' + file));
      };
      document.head.appendChild(el);
    });
    return loading[file];
  }

  function ensure(kind) {
    switch (kind) {
      case 'math':
      case 'math-inline':
        // mhchem registers \ce and \pu onto KaTeX, so it loads after it.
        return Promise.all([
          load('katex.min.css', true),
          load('katex.min.js').then(function () { return load('mhchem.min.js'); }),
        ]);
      case 'mermaid':
        return load('mermaid.min.js');
      case 'graphviz':
        return load('viz-global.js');
      case 'plot':
        return load('plot.js');
      default:
        return Promise.reject(new Error('unknown kind ' + kind));
    }
  }

  // Graphviz draws in plain black on a transparent ground. It is recoloured to the ink
  // here, in CSS, and NOT by passing colours to the engine: a colour or font handed to
  // Graphviz changes the metrics it lays the graph out with. An author's own colours
  // survive — these rules match only what the engine writes when nothing was asked for.
  // (md's MarkdownHTML.css, the same four rules.)
  (function style() {
    var css =
      '.fc-rich{display:inline-block;box-sizing:border-box;line-height:normal;vertical-align:top}' +
      '.fc-rich-card{padding:' + CARD_PADDING + 'px}' +
      '.fc-rich svg{display:block}' +
      '.fc-rich-graphviz svg text:not([fill]){fill:var(--fc-ink)}' +
      '.fc-rich-graphviz svg text[fill="black"]{fill:var(--fc-ink)}' +
      '.fc-rich-graphviz svg [stroke="black"]{stroke:var(--fc-ink)}' +
      '.fc-rich-graphviz svg [fill="black"]:not(text){fill:var(--fc-ink)}' +
      '.fc-rich .katex-display{margin:0}' +
      '.fc-rich-baseline{display:inline-block;width:0;height:0;vertical-align:baseline}';
    var el = document.createElement('style');
    el.textContent = css;
    (document.head || document.documentElement).appendChild(el);
  })();

  var mermaidTheme = null;
  var mermaidSeq = 0;
  var vizInstance = null;

  function fail(message) {
    return { ok: false, error: String(message || 'error') };
  }

  // A diagram's SVG at its natural size, scaled down (never up) to fit the job's box.
  function sizeSvg(svg, job) {
    var w = 0, h = 0;
    var vb = svg.viewBox && svg.viewBox.baseVal;
    if (vb && vb.width > 0 && vb.height > 0) { w = vb.width; h = vb.height; }
    if (!(w > 0 && h > 0)) {
      var r = svg.getBoundingClientRect();
      w = r.width; h = r.height;
    }
    if (!(w > 0 && h > 0)) return false;
    var maxW = (job.maxWidth || 1200) - 2 * CARD_PADDING;
    var maxH = (job.maxHeight || 3000) - 2 * CARD_PADDING;
    var scale = Math.min(1, maxW / w, maxH / h);
    svg.removeAttribute('style');
    svg.setAttribute('width', String(Math.max(1, Math.floor(w * scale))));
    svg.setAttribute('height', String(Math.max(1, Math.floor(h * scale))));
    return true;
  }

  function drawMath(el, job) {
    var display = job.kind === 'math';
    katex.render(job.source, el, {
      displayMode: display,
      throwOnError: true,
      trust: false,
      strict: 'ignore',
      maxSize: 40,
      maxExpand: 500,
      output: job.mathml ? 'htmlAndMathml' : 'html',
    });
    var marker = document.createElement('span');
    marker.className = 'fc-rich-baseline';
    // The baseline of the formula's own line: the marker sits on it.
    var line = display ? el.querySelector('.katex-html') || el : el;
    line.appendChild(marker);
    return marker;
  }

  function drawMermaid(el, job) {
    var theme = job.dark ? 'dark' : 'default';
    if (mermaidTheme !== theme) {
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: theme,
        // A chat bubble is small: keep the text at the size the rest of it reads at.
        fontSize: job.fontSize || 16,
      });
      mermaidTheme = theme;
    }
    var id = 'fc-mermaid-' + (++mermaidSeq);
    return mermaid.render(id, job.source).then(
      function (out) {
        el.innerHTML = out.svg;
        var svg = el.querySelector('svg');
        if (!svg) throw new Error('empty');
        if (!sizeSvg(svg, job)) throw new Error('empty');
      },
      function (err) {
        // A failed render leaves its error drawing in the body; take it away.
        var stray = document.getElementById('d' + id);
        if (stray && stray.parentNode) stray.parentNode.removeChild(stray);
        throw err;
      });
  }

  function drawGraphviz(el, job) {
    var engine = GRAPHVIZ_ENGINES.indexOf(job.engine) >= 0 ? job.engine : 'dot';
    var instance = vizInstance || (vizInstance = Viz.instance());
    return instance.then(function (viz) {
      var svg = viz.renderSVGElement(job.source, {
        engine: engine,
        graphAttributes: { bgcolor: 'transparent' },
      });
      el.appendChild(svg);
      if (!sizeSvg(svg, job)) throw new Error('empty');
    });
  }

  function drawPlot(el, job) {
    var svgText = fcPlot.plotSVG(job.source);
    if (!svgText) throw new Error('empty');
    el.innerHTML = svgText;
    var svg = el.querySelector('svg');
    if (!svg || !sizeSvg(svg, job)) throw new Error('empty');
  }

  /** Draws `job` into `el`, replacing what was there. Resolves to a RESULT; never rejects. */
  function renderInto(el, job) {
    job = job || {};
    var kind = job.kind;
    var source = typeof job.source === 'string' ? job.source : '';
    if (!source.trim()) return Promise.resolve(fail('empty'));
    var isMath = kind === 'math' || kind === 'math-inline';
    el.textContent = '';
    el.className = 'fc-rich fc-rich-' + kind + (isMath ? '' : ' fc-rich-card');
    el.style.color = job.color || '';
    el.style.background = job.background || 'transparent';
    el.style.setProperty('--fc-ink', job.color || 'currentColor');
    if (isMath && job.fontSize) el.style.fontSize = job.fontSize + 'px';
    var marker = null;
    return ensure(kind)
      .then(function () {
        switch (kind) {
          case 'math':
          case 'math-inline':
            marker = drawMath(el, job);
            return null;
          case 'mermaid': return drawMermaid(el, job);
          case 'graphviz': return drawGraphviz(el, job);
          case 'plot': return drawPlot(el, job);
        }
        throw new Error('unknown kind ' + kind);
      })
      .then(function () {
        var box = el.getBoundingClientRect();
        var width = Math.ceil(box.width), height = Math.ceil(box.height);
        if (!(width > 0 && height > 0)) throw new Error('empty');
        var baseline = height;
        if (marker) {
          baseline = marker.getBoundingClientRect().top - box.top;
          marker.parentNode.removeChild(marker);
        }
        return { ok: true, width: width, height: height, baseline: baseline };
      })
      .catch(function (err) {
        el.textContent = '';
        return fail(err && err.message ? err.message : err);
      });
  }

  // Native hosts draw one job at a time on the page's one stage, which sits at (0, 0).
  var queue = Promise.resolve();

  /** Native: draws `job` on `#stage` and resolves to its RESULT. Jobs run one after another. */
  function draw(job) {
    var run = queue.then(function () {
      var stage = document.getElementById('stage');
      if (!stage) return fail('no stage');
      return renderInto(stage, job);
    });
    queue = run.then(function () {}, function () {});
    return run;
  }

  /**
   * Native hosts whose script call cannot wait for a promise (Android's WebView, WebView2):
   * draws `job`, then posts {id, result} as JSON to whichever bridge the page has —
   * `FcRichBridge.done(json)` on Android, `chrome.webview.postMessage(json)` in WebView2,
   * `webkit.messageHandlers.fcRich` on Apple.
   */
  function drawAndPost(id, job) {
    return draw(job).then(function (result) {
      var json = JSON.stringify({ id: id, result: result });
      try { if (window.FcRichBridge) { window.FcRichBridge.done(json); return; } } catch (e) {}
      try { if (window.chrome && window.chrome.webview) { window.chrome.webview.postMessage(json); return; } } catch (e) {}
      try { window.webkit.messageHandlers.fcRich.postMessage(json); } catch (e) {}
    });
  }

  /** Native: warms the engines a job of `kind` needs, so the first bubble does not wait for them. */
  function prepare(kind) {
    return ensure(kind).then(function () { return true; }, function () { return false; });
  }

  window.fcRich = {
    version: VERSION,
    renderInto: renderInto,
    draw: draw,
    drawAndPost: drawAndPost,
    prepare: prepare,
  };
})();
