import fs from 'fs';
import vm from 'vm';
const src = fs.readFileSync(process.argv[2], 'utf8');
function run(hostname) {
  class XHR { get response() { return this._r } get responseType() { return this._t } }
  class R extends Response {} ; const ctx = { location: { hostname }, Response: R, XMLHttpRequest: XHR, console };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(src, ctx);
  return ctx;
}
const player = () => ({ videoDetails: { id: 1 }, streamingData: { f: 1 }, adPlacements: [1], adSlots: [2], playerAds: [3], adBreakHeartbeatParams: 'x' });
let ok = true; const check = (c, m) => { if (!c) { ok = false; console.log('FAIL', m) } };
const c = run('www.youtube.com');
vm.runInContext('var ytInitialPlayerResponse = ' + JSON.stringify(player()), c);
const p = vm.runInContext('ytInitialPlayerResponse', c);
check(p.streamingData && p.videoDetails && !('adPlacements' in p) && !('adSlots' in p) && !('playerAds' in p), 'initial ' + JSON.stringify(p));
const parsed = vm.runInContext(`JSON.parse('${JSON.stringify([{ playerResponse: player() }, { x: 1 }])}')`, c);
check(!('adPlacements' in parsed[0].playerResponse) && parsed[0].playerResponse.streamingData && parsed[1].x === 1, 'array');
const shorts = vm.runInContext(`JSON.parse('${JSON.stringify({ entries: [{ command: { reelWatchEndpoint: { adClientParams: { isAd: true } } } }, { command: { reelWatchEndpoint: { videoId: 'a' } } }] })}')`, c);
check(shorts.entries.length === 1 && shorts.entries[0].command.reelWatchEndpoint.videoId === 'a', 'shorts');
const fromResponse = await vm.runInContext(`new Response(JSON.stringify(${JSON.stringify(player())})).json()`, c);
check(!('adSlots' in fromResponse) && fromResponse.streamingData, 'response.json');
const x = new c.XMLHttpRequest(); x._t = 'json'; x._r = player();
check(!('playerAds' in x.response), 'xhr json');
const y = new c.XMLHttpRequest(); y._t = 'text'; y._r = 'adSlots';
check(y.response === 'adSlots', 'xhr text untouched');
vm.runInContext(`Object.prototype.hasOwnProperty = () => { throw new Error('poisoned') }; Reflect.apply = () => { throw 1 };`, c);
const after = vm.runInContext(`JSON.parse('{"adSlots":1,"a":2}')`, c);
check(!('adSlots' in after) && after.a === 2, 'survives poisoned builtins ' + JSON.stringify(after));
check(vm.runInContext(`JSON.parse('"text"')`, c) === 'text', 'primitive');
for (const h of ['example.com', 'notyoutube.com', 'youtube.com.evil.example', 'accounts.youtube.com', '']) {
  const o = run(h);
  check(vm.runInContext("JSON.parse.toString().includes(\"native code\")", o), "untouched " + h);
}
const other = run('evil-youtube.com');
vm.runInContext('var ytInitialPlayerResponse = ' + JSON.stringify(player()), other);
check('adPlacements' in vm.runInContext('ytInitialPlayerResponse', other), 'other host untouched');
for (const h of ['m.youtube.com', 'youtube-nocookie.com', 'www.youtubekids.com', 'youtube.com']) {
  const o = run(h); vm.runInContext('var ytInitialPlayerResponse = ' + JSON.stringify(player()), o);
  check(!('adPlacements' in vm.runInContext('ytInitialPlayerResponse', o)), 'host ' + h);
}
console.log(ok ? 'ALL OK' : 'FAILED');
