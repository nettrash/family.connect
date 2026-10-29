// Phase-0 spike for issue #112: Chromium's own WebRTC against the GStreamer peer.
import { chromium } from 'playwright';
const mode = process.argv[2];
setTimeout(() => { console.log('SPIKE TIMEOUT'); process.exit(2); }, 45000).unref(); // browser-offers-video | browser-offers-audio | browser-answers
const page_js = `
async function main(mode) {
  const log = (t) => console.log(t);
  const stream = await navigator.mediaDevices.getUserMedia({audio: true, video: mode !== 'browser-offers-audio'});
  const pc = new RTCPeerConnection({iceServers: []});
  stream.getTracks().forEach(t => pc.addTrack(t, stream));
  if (mode === 'browser-offers-audio') pc.addTransceiver('video', {direction: 'recvonly'});  // the web client's audio call
  const ws = new WebSocket('ws://rtcpeer:9000');
  let remoteSet = false; const pending = [];
  pc.onicecandidate = e => { if (e.candidate) ws.send(JSON.stringify({type: 'ice', sdpMLineIndex: e.candidate.sdpMLineIndex, candidate: e.candidate.candidate})); };
  pc.onconnectionstatechange = () => log('chromium connection ' + pc.connectionState);
  ws.onmessage = async m => {
    const d = JSON.parse(m.data);
    if (d.type === 'offer') {
      await pc.setRemoteDescription({type: 'offer', sdp: d.sdp}); remoteSet = true;
      for (const c of pending.splice(0)) await pc.addIceCandidate(c);
      const a = await pc.createAnswer(); await pc.setLocalDescription(a);
      ws.send(JSON.stringify({type: 'answer', sdp: a.sdp}));
    } else if (d.type === 'answer') {
      await pc.setRemoteDescription({type: 'answer', sdp: d.sdp}); remoteSet = true;
      for (const c of pending.splice(0)) await pc.addIceCandidate(c);
    } else if (d.type === 'ice') {
      const c = {candidate: d.candidate, sdpMLineIndex: d.sdpMLineIndex};
      if (remoteSet) await pc.addIceCandidate(c); else pending.push(c);
    }
  };
  await new Promise(r => ws.onopen = r);
  if (mode.startsWith('browser-offers')) {
    const o = await pc.createOffer(); await pc.setLocalDescription(o);
    ws.send(JSON.stringify({type: 'offer', sdp: o.sdp}));
  }
  await new Promise(r => setTimeout(r, 14000));
  let audio = 0, video = 0, codecs = [];
  (await pc.getStats()).forEach(r => {
    if (r.type === 'inbound-rtp' && r.kind === 'audio') audio = r.packetsReceived;
    if (r.type === 'inbound-rtp' && r.kind === 'video') video = r.framesDecoded;
    if (r.type === 'codec') codecs.push(r.mimeType);
  });
  return {state: pc.connectionState, audioPackets: audio, videoFrames: video, codecs: [...new Set(codecs)]};
}`;
const browser = await chromium.launch({args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--disable-features=WebRtcHideLocalIpsWithMdns']});
const page = await browser.newPage();
page.on('console', m => console.log('page:', m.text()));
await page.route('http://localhost/spike', r => r.fulfill({contentType: 'text/html', body: '<html><body>spike</body></html>'}));
await page.goto('http://localhost/spike');
await page.addScriptTag({content: page_js});
const result = await page.evaluate(m => main(m), mode);
console.log('CHROMIUM RESULT', JSON.stringify(result), 'version', browser.version());
await browser.close();
const wantVideo = mode !== 'browser-offers-audio';
process.exit(result.state === 'connected' && result.audioPackets > 50 && (!wantVideo || result.videoFrames > 50) ? 0 : 1);
