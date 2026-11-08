// Runs the app's own core modules (copied from mobile/harmony, kits shimmed)
// against the DDB1 vectors and then against a live bridge + test core.
// Usage (from tests/integration/harmony-logic): node main.ts <vectors.json> <pair link> <agent t> <cat t>
import { readFileSync } from 'node:fs';
import { b64urlDecode, b64urlEncode, equal, utf8, fromUtf8 } from './Bytes.ts';
import { clientKeys, x25519Private, XKey, Records } from './Crypto.ts';
import { parsePairLink } from './Link.ts';
import { Session, Conn } from './Session.ts';
import { Group } from './Agents.ts';

let fail = 0;
setTimeout(() => { console.log('FAIL timed out (60 s)'); process.exit(1); }, 60000).unref();
function check(name: string, ok: boolean, extra = ''): void {
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${extra ? ' — ' + extra : ''}`);
  if (!ok) fail = 1;
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(name: string, cond: () => boolean, ms = 8000): Promise<boolean> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (cond()) return true;
    await sleep(50);
  }
  return false;
}

const [vectorsPath, link, agentT, catT] = process.argv.slice(2);

// 1. Fixed vectors from bridge/src/crypto.rs.
const v = JSON.parse(readFileSync(vectorsPath, 'utf8'));
const ce = new XKey(x25519Private(b64urlDecode(v.client_secret)), b64urlDecode(v.client_public));
const keys = clientKeys(ce, b64urlDecode(v.bridge_public), b64urlDecode(v.msg2));
check('vectors: first c2s record matches the bridge', b64urlEncode(keys.send.seal(b64urlDecode(v.c2s_0_plain))) === v.c2s_0_sealed);
check('vectors: first s2c record opens', equal(keys.recv.open(b64urlDecode(v.s2c_0_sealed)), b64urlDecode(v.s2c_0_plain)));
const raw = new Uint8Array([0xfb, 0xff, 0x00, 0x10, 0x83, 0xff, 0xfe]);
check('base64url round trip', equal(b64urlDecode(b64urlEncode(raw)), raw) && b64urlEncode(raw) === Buffer.from(raw).toString('base64url'));
check('utf8 round trip', fromUtf8(utf8('需要你处理 ✓')) === '需要你处理 ✓');
const recs = new Records();
recs.push(new Uint8Array([0, 0, 0, 2, 7]));
check('records wait for a whole record', recs.pop() === null);
recs.push(new Uint8Array([8, 0, 0]));
const one = recs.pop();
check('records split correctly', one !== null && equal(one, new Uint8Array([7, 8])) && recs.pop() === null);

// 2. The session against the live bridge.
const s = Session.shared;
await s.start(); // nothing saved yet
check('no machine before pairing', s.machine === null);
await s.pair(parsePairLink(link));
check('pairing connects', s.conn === Conn.Ready, s.error);
check('core version known', s.coreVersion.length > 0, s.coreVersion);
check('agent visible in the list', await until('agent', () => s.agents.get(agentT) !== undefined));
check('fake agent groups as needing you, working, done or idle', await until('status', () => {
  const a = s.agents.get(agentT);
  return a !== undefined && a.group() !== Group.Shell;
}), String(s.agents.get(agentT)?.group()));
const seenBlocked = await until('blocked', () => s.agents.get(agentT)?.group() === Group.NeedsYou, 8000);
check('a blocked agent shows under needs you', seenBlocked);

// Attach to the cat terminal and type into it through terminal.send.
const cat = s.agents.get(catT);
check('cat terminal listed as a shell', cat !== undefined && cat.group() === Group.Shell);
let screen = '';
let resized = false;
const ch = await s.attach(cat!, 60, 20, {
  data: (b: Uint8Array) => { screen += fromUtf8(b); },
  resize: () => { resized = true; },
  closed: () => {},
});
check('attach opens a channel', ch > 0, String(ch));
check('phone width: the terminal takes the phone size', await until('size', () => s.agents.get(catT)?.cols === 60 && s.agents.get(catT)?.rows === 20), `${s.agents.get(catT)?.cols}x${s.agents.get(catT)?.rows}`);
s.resize(ch, 50, 18);
check('a viewport change resizes it', await until('resize', () => s.agents.get(catT)?.cols === 50 && s.agents.get(catT)?.rows === 18), `${s.agents.get(catT)?.cols}x${s.agents.get(catT)?.rows}`);
await s.sendText(catT, 'hello-from-harmony', false);
check('sent text comes back on the stream', await until('echo', () => screen.includes('hello-from-harmony')));
await s.keys(catT, ['ctrl+c']);
s.detach(ch);

// herdr.seen goes up when a person looks.
const a = s.agents.get(agentT)!;
const turnBefore = a.turn;
await until('turn', () => (s.agents.get(agentT)?.turn ?? 0) > 0, 8000);
const ag = s.agents.get(agentT)!;
ag.seen = -1;
s.markSeen(ag);
check('markSeen writes herdr.seen', await until('seen label', () => (s.agents.get(agentT)?.seen ?? -1) === ag.turn), `turn ${turnBefore}->${ag.turn}`);

// Reconnect with the saved device token (the auth path).
await s.connect();
check('saved device token reconnects', s.conn === Conn.Ready, s.error);
check('events flow again after reconnect', await until('agents', () => s.agents.get(agentT) !== undefined));

s.setForeground(false);
await s.forget();
check('forget clears the machine', s.machine === null && s.conn === Conn.Idle);

process.exit(fail);
