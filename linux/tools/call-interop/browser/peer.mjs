// The browser peer of the call interop harness (issue #112, P6): Chromium or Firefox through
// Playwright, with fake camera and microphone, speaking the protocol's call frames over the relay
// exactly as the web client does (web/src/calls.rs: place, answer, candidates held until the
// remote description, `call_end` with the stage's reason, mute and camera as `track.enabled`).
//
//   node peer.mjs '<json options>'
//
// Options: label, browser ("chromium" | "firefox"), relay, user, peer, chat, place (null or
// {video, recvonlyVideo, h264Only}), placeAfter, answerAfter, talk, mute [a,b], cameraOff [a,b],
// peerMute [a,b], peerCameraOff [a,b], timeout, mdns (keep the browser's mDNS host candidates),
// iceServers, expectAudio, expectVideo, expectNoVideo, expectCodec, expectEnd.
import { chromium, firefox } from 'playwright';

const o = JSON.parse(process.argv[2]);
setTimeout(() => { console.log(`[${o.label}] BROWSER TIMEOUT`); process.exit(3); }, (o.timeout + 30) * 1000).unref();

const pageScript = String.raw`
function summary(sdp) {
  const out = [];
  let s = null;
  for (const line of sdp.split(/\r?\n/)) {
    if (line.startsWith('m=')) {
      const w = line.slice(2).split(' ');
      s = { kind: w[0], zero: w[1] === '0', pts: w.slice(3), names: {}, dir: 'sendrecv', only: false };
      out.push(s);
    } else if (s) {
      const m = line.match(/^a=rtpmap:(\d+) ([^/]+)/);
      if (m) s.names[m[1]] = m[2];
      else if (line === 'a=bundle-only') s.only = true;
      else if (/^a=(sendrecv|sendonly|recvonly|inactive)$/.test(line)) s.dir = line.slice(2);
    }
  }
  return out.map(s => {
    const codecs = s.pts.map(p => s.names[p] ? s.names[p] + '/' + p : null)
      .filter(c => c && /^(opus|vp8|vp9|h264|av1)/i.test(c));
    const state = s.zero ? (s.only ? s.dir + '(bundle-only)' : 'rejected') : s.dir;
    return s.kind + ' ' + state + ' ' + codecs.join(',');
  }).join(' | ');
}

async function main(o) {
  const t0 = performance.now();
  const now = () => Math.round((performance.now() - t0) / 100) / 10;
  const log = (t) => console.log(now() + ' ' + t);
  const report = { label: o.label, browser: navigator.userAgent, stages: [], samples: [], said: [],
                   ended: null, local_sdp: null, remote_sdp: null, codecs: {} };
  let call = null;
  let peak = 0;
  let done;
  const finished = new Promise(resolve => done = resolve);

  const ws = new WebSocket(o.relay + '/?user=' + o.user);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  const say = (frame) => { report.said.push([now(), frame.type]); ws.send(JSON.stringify(frame)); };
  const stage = (c, s) => { c.stage = s; report.stages.push([now(), s]); log('stage ' + s); };

  // What this tab sends: the camera too on a video call, else (or when refused) the microphone.
  async function media(video) {
    try {
      return { stream: await navigator.mediaDevices.getUserMedia({ audio: true, video }), camera: video };
    } catch (e) {
      if (!video) throw e;
      return { stream: await navigator.mediaDevices.getUserMedia({ audio: true, video: false }), camera: false };
    }
  }

  // The far side's sound, played (Chromium feeds Web Audio from a remote track only while an
  // element plays it) and measured.
  let analysing = false;
  function listen(remote) {
    if (analysing || remote.getAudioTracks().length === 0) return;
    analysing = true;
    const element = new Audio();
    element.srcObject = remote;
    element.play().catch(() => {});
    const context = new AudioContext();
    context.resume().catch(() => {});
    const analyser = context.createAnalyser();
    analyser.fftSize = 2048;
    context.createMediaStreamSource(remote).connect(analyser);
    const data = new Float32Array(analyser.fftSize);
    setInterval(() => {
      analyser.getFloatTimeDomainData(data);
      for (const v of data) peak = Math.max(peak, Math.abs(v));
    }, 40);
  }

  function connect(c, stream) {
    const pc = new RTCPeerConnection({ iceServers: o.iceServers || [] });
    for (const track of stream.getTracks()) pc.addTrack(track, stream);
    pc.onicecandidate = e => {
      // The last candidate is a null one: gathering is done, nothing to relay.
      if (e.candidate) say({ type: 'call_ice', call_id: c.id, candidate: {
        candidate: e.candidate.candidate, sdp_mid: e.candidate.sdpMid, sdp_mline_index: e.candidate.sdpMLineIndex } });
    };
    const remote = new MediaStream();
    pc.ontrack = e => { remote.addTrack(e.track); listen(remote); };
    pc.onconnectionstatechange = () => {
      log('connection ' + pc.connectionState);
      if (pc.connectionState === 'connected') up(c);
      if (pc.connectionState === 'failed' && call === c && c.stage !== 'Ended') finish(c, 'failed', 'failed');
    };
    c.pc = pc; c.local = stream; c.remoteSet = false; c.early = c.early || [];
    return pc;
  }

  // A video call with no camera of our own still negotiates the far side's picture.
  const receiveVideo = (pc) => pc.addTransceiver('video', { direction: 'recvonly' });

  function preferH264(pc) {
    const codecs = RTCRtpReceiver.getCapabilities('video').codecs.filter(c => c.mimeType === 'video/H264');
    for (const t of pc.getTransceivers()) {
      if (t.receiver.track && t.receiver.track.kind === 'video') t.setCodecPreferences(codecs);
    }
  }

  async function remoteIsSet(c) {
    c.remoteSet = true;
    for (const candidate of c.early.splice(0)) await c.pc.addIceCandidate(candidate).catch(e => log('candidate refused: ' + e.name));
  }

  function up(c) {
    if (c.stage !== 'Connecting') return;
    stage(c, 'Talking');
    c.talkAt = now();
    const tracks = (kind) => c.local.getTracks().filter(t => t.kind === kind);
    const at = (s, f) => setTimeout(f, s * 1000);
    if (o.mute) { at(o.mute[0], () => tracks('audio').forEach(t => t.enabled = false)); at(o.mute[1], () => tracks('audio').forEach(t => t.enabled = true)); }
    if (o.cameraOff) { at(o.cameraOff[0], () => tracks('video').forEach(t => t.enabled = false)); at(o.cameraOff[1], () => tracks('video').forEach(t => t.enabled = true)); }
    if (o.talk) at(o.talk, () => { log('hanging up'); finish(c, 'hangup', 'hangup'); });
  }

  function finish(c, shown, sayReason) {
    if (c.stage === 'Ended') return;
    if (sayReason) say({ type: 'call_end', call_id: c.id, reason: sayReason });
    stage(c, 'Ended');
    report.ended = [now(), shown];
    sample();
    if (c.pc) c.pc.close();
    if (c.local) c.local.getTracks().forEach(t => t.stop());
    setTimeout(() => done(), 2000);
  }

  async function place(p) {
    const c = { id: crypto.randomUUID(), video: p.video, outgoing: true, early: [] };
    call = c;
    stage(c, 'Dialling');
    const m = await media(p.video);
    const pc = connect(c, m.stream);
    if (p.video && !m.camera) receiveVideo(pc);
    if (p.recvonlyVideo) receiveVideo(pc);  // the web client's old voice call: audio + a recvonly video
    if (p.h264Only) preferH264(pc);
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    report.local_sdp = summary(offer.sdp);
    if (o.dumpOffer) report.offer_sdp = offer.sdp;
    const frame = { type: 'call_offer', call_id: c.id, chat_id: o.chat, sdp: offer.sdp };
    if (p.video) frame.video = true;
    say(frame);
  }

  async function answer(c) {
    const m = await media(c.video);
    const pc = connect(c, m.stream);
    if (c.video && !m.camera) receiveVideo(pc);
    await pc.setRemoteDescription({ type: 'offer', sdp: c.offer });
    report.remote_sdp = summary(c.offer);
    await remoteIsSet(c);
    const a = await pc.createAnswer();
    await pc.setLocalDescription(a);
    report.local_sdp = summary(a.sdp);
    if (o.dumpOffer) { report.offer_sdp = c.offer; report.answer_sdp = a.sdp; }
    say({ type: 'call_answer', call_id: c.id, sdp: a.sdp });
    stage(c, 'Connecting');
    if (pc.connectionState === 'connected') up(c);
  }

  ws.onmessage = async (message) => {
    const f = JSON.parse(message.data);
    if (f.type === 'call_offer') {
      if (call) return;
      const c = { id: f.call_id, video: !!f.video, outgoing: false, offer: f.sdp, early: [] };
      call = c;
      stage(c, 'Incoming');
      setTimeout(() => answer(c).catch(e => { log('answer failed: ' + e); finish(c, 'failed', 'failed'); }), o.answerAfter * 1000);
      return;
    }
    const c = call;
    if (!c || f.call_id !== c.id) return;
    if (f.type === 'call_ringing' && c.stage === 'Dialling') stage(c, 'Ringing');
    else if (f.type === 'call_answer' && c.outgoing) {
      await c.pc.setRemoteDescription({ type: 'answer', sdp: f.sdp });
      report.remote_sdp = summary(f.sdp);
      await remoteIsSet(c);
      stage(c, 'Connecting');
      if (c.pc.connectionState === 'connected') up(c);
    } else if (f.type === 'call_ice') {
      const candidate = { candidate: f.candidate.candidate };
      if (f.candidate.sdp_mid != null) candidate.sdpMid = f.candidate.sdp_mid;
      if (f.candidate.sdp_mline_index != null) candidate.sdpMLineIndex = f.candidate.sdp_mline_index;
      if (c.remoteSet) await c.pc.addIceCandidate(candidate).catch(e => log('candidate refused: ' + e.name));
      else c.early.push(candidate);
    } else if (f.type === 'call_end') {
      finish(c, f.reason, null);
    }
  };

  async function sample() {
    const c = call;
    const row = { t: now(), talk: c && c.talkAt != null ? Math.round((now() - c.talkAt) * 10) / 10 : null,
                  stage: c ? c.stage : '-', pc: c && c.pc ? c.pc.connectionState : '-',
                  aIn: 0, vIn: 0, vDecoded: 0, aOut: 0, vOut: 0, peak: Math.round(peak * 32767) };
    peak = 0;
    if (c && c.pc && c.pc.connectionState !== 'closed') {
      const stats = await c.pc.getStats();
      const codecs = {};
      stats.forEach(r => { if (r.type === 'codec') codecs[r.id] = r.mimeType + '/' + r.payloadType; });
      stats.forEach(r => {
        if (r.type === 'inbound-rtp' && r.kind === 'audio') { row.aIn = r.packetsReceived; report.codecs.audioIn = codecs[r.codecId]; }
        if (r.type === 'inbound-rtp' && r.kind === 'video') { row.vIn = r.packetsReceived; row.vDecoded = r.framesDecoded || 0; report.codecs.videoIn = codecs[r.codecId]; }
        if (r.type === 'outbound-rtp' && r.kind === 'audio') { row.aOut = r.packetsSent; report.codecs.audioOut = codecs[r.codecId]; }
        if (r.type === 'outbound-rtp' && r.kind === 'video') { row.vOut = r.packetsSent; report.codecs.videoOut = codecs[r.codecId]; }
      });
      report.samples.push(row);
    } else {
      report.samples.push(row);
    }
  }
  const sampler = setInterval(sample, 1000);

  if (o.place) setTimeout(() => place(o.place).catch(e => { log('place failed: ' + e); done(); }), o.placeAfter * 1000);
  setTimeout(() => { report.timedOut = true; done(); }, o.timeout * 1000);
  await finished;
  clearInterval(sampler);
  ws.close();
  return report;
}
`;

const browser = o.browser === 'firefox'
  ? await firefox.launch({ firefoxUserPrefs: {
      'media.navigator.streams.fake': true,
      'media.navigator.permission.disabled': true,
      'media.peerconnection.ice.obfuscate_host_addresses': !!o.mdns,
      'media.autoplay.default': 0,
      'media.autoplay.block-webaudio': false,
    } })
  : await chromium.launch({ args: [
      '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
      '--autoplay-policy=no-user-gesture-required',
      ...(o.mdns ? [] : ['--disable-features=WebRtcHideLocalIpsWithMdns']),
    ] });
const page = await browser.newPage();
page.on('console', m => console.log(`[${o.label}] page: ${m.text()}`));
await page.route('http://localhost/peer', r => r.fulfill({ contentType: 'text/html', body: '<html><body>peer</body></html>' }));
await page.goto('http://localhost/peer');
await page.addScriptTag({ content: pageScript });
const r = await page.evaluate(options => main(options), o);
console.log('BROWSER REPORT ' + JSON.stringify(r));
console.log(`[${o.label}] ${o.browser} ${browser.version()}`);

// ---- the checks -----------------------------------------------------------------------------
const checks = [];
const check = (name, passed, detail) => checks.push([name, passed, detail]);
const max = (field) => Math.max(0, ...r.samples.map(s => s[field] || 0));
const windowed = (a, b) => r.samples.filter(s => s.talk != null && s.talk >= a && s.talk <= b);
const delta = (rows, field) => rows.length ? rows[rows.length - 1][field] - rows[0][field] : 0;
const peakOf = (rows) => Math.max(0, ...rows.map(s => s.peak));
check('connected', r.stages.some(s => s[1] === 'Talking'), JSON.stringify(r.stages));
if (o.expectAudio != null) check('far audio received', max('aIn') >= o.expectAudio, `${max('aIn')} packets, codec ${r.codecs.audioIn}`);
if (o.expectVideo != null) check('far video decoded', max('vDecoded') >= o.expectVideo, `${max('vDecoded')} frames (${max('vIn')} packets), codec ${r.codecs.videoIn}`);
if (o.expectNoVideo) check('no far video', max('vIn') === 0, `${max('vIn')} packets`);
if (o.expectCodec) check('far video codec', (r.codecs.videoIn || '').toUpperCase().includes(o.expectCodec.toUpperCase()), `${r.codecs.videoIn} in, ${r.codecs.videoOut} out`);
if (o.expectEnd) check('ended with', r.ended && r.ended[1] === o.expectEnd, JSON.stringify(r.ended));
if (o.peerMute) {
  const [a, b] = o.peerMute;
  const before = windowed(1, a - 0.5), during = windowed(a + 1.5, b - 0.2), after = windowed(b + 1.5, b + 4);
  const loud = peakOf(before), quiet = peakOf(during), again = peakOf(after);
  check('far mute: rtp keeps flowing, sound stops', delta(during, 'aIn') > 0 && quiet * 10 < Math.max(loud, 1) && again * 10 > loud,
        `rtp +${delta(during, 'aIn')} while muted; peak before ${loud}, muted ${quiet}, after ${again}`);
}
if (o.peerCameraOff) {
  const [a, b] = o.peerCameraOff;
  const before = windowed(1, a - 0.2), during = windowed(a + 1.5, b - 0.2), after = windowed(b + 1.5, b + 4);
  check('far camera off: video stops, then resumes', delta(before, 'vIn') > 0 && delta(during, 'vIn') === 0 && delta(after, 'vIn') > 0,
        `video packets +${delta(before, 'vIn')} before, +${delta(during, 'vIn')} while off, +${delta(after, 'vIn')} after`);
}
if (r.timedOut) check('finished in time', false, 'timed out');
let failed = false;
for (const [name, passed, detail] of checks) {
  if (!passed) failed = true;
  console.log(`[${o.label}] CHECK ${passed ? 'PASS' : 'FAIL'}: ${name} — ${detail}`);
}
console.log(`[${o.label}] RESULT ${failed ? 'FAIL' : 'PASS'}`);
await browser.close();
process.exit(failed ? 1 : 0);
