const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/screen-presence.js'), 'utf8');
const context = {module: {exports: {}}};
vm.runInNewContext(source, context);
const api = context.module.exports;
const active = {enabled: true, known: true, screensaver: false, locked: false, sleeping: false, session_active: true};
function state(extra = {}) {return {windowState: 'Background', suspendedState: 'Unknown', machineLockState: 'Unknown', machineIdleState: 'Idle', ...extra};}
class Subject {
  constructor(value) {this.value = value; this.events = [];}
  getValue() {if (this.unavailable) throw Error('provider disposed'); return this.value;}
  next(value) {this.value = value; this.events.push(value);}
}
function fixture(value = state()) {
  const subject = new Subject(value), timers = new Map(); let scans = 0;
  const host = {_hostStateSubject: subject, getState() {return {...subject.value};}, observe() {return {};},
    get preferredAvailability() {throw Error('must not inspect or set manual presence');}};
  const adapter = api.createAdapter({findHost() {scans++; return host;}, setTimer(callback) {timers.set(1, callback); return 1;}, clearTimer(id) {timers.delete(id);}});
  return {subject, host, adapter, timers, scans: () => scans};
}

test('disabled/unknown native state never inspects services or overrides Teams', () => {
  const f = fixture(); f.adapter.receive({...active, enabled: false}); f.adapter.receive({...active, known: false});
  assert.equal(f.scans(), 0); assert.equal(f.adapter.getStats().hooked, false); assert.equal(f.subject.value.machineIdleState, 'Idle');
});
test('explicit screen policy stays active despite no in-page activity, without input or focus spoofing', () => {
  const f = fixture(); f.adapter.receive(active); assert.equal(f.subject.value.machineIdleState, 'Active');
  const incoming = state({windowState: 'Minimized'}); f.subject.next(incoming);
  assert.equal(f.subject.value.machineIdleState, 'Active'); assert.equal(f.subject.value.windowState, 'Minimized');
  assert.equal(f.subject.value.suspendedState, 'Unknown'); assert.equal(incoming.machineIdleState, 'Idle');
  assert.doesNotMatch(source, /mousemove|keydown|dispatchEvent|document\.hasFocus|setUserPreferredPresence|setAvailability|acquireToken/);
});
test('screen saver, lock and sleeping/session-off are inactive; wake alone never unlocks the screen', () => {
  const f = fixture(state({machineIdleState: 'Active'})); f.adapter.receive(active);
  f.adapter.receive({...active, screensaver: true}); assert.equal(f.subject.value.machineIdleState, 'Idle'); assert.equal(f.subject.value.machineLockState, 'Active');
  f.adapter.receive({...active, locked: true}); assert.equal(f.subject.value.machineLockState, 'Locked');
  f.adapter.receive({...active, locked: true, sleeping: false}); assert.equal(f.subject.value.machineIdleState, 'Idle');
  f.adapter.receive(active); assert.equal(f.subject.value.machineIdleState, 'Active');
  f.adapter.receive({...active, sleeping: true}); assert.equal(f.subject.value.machineIdleState, 'Idle');
  f.adapter.receive({...active, session_active: false}); assert.equal(f.subject.value.machineIdleState, 'Idle');
});
test('existing genuine suspension is preserved and preferred-presence APIs are not touched', () => {
  const original = state({suspendedState: 'Suspending'}); const f = fixture(original); f.adapter.receive(active);
  assert.equal(f.subject.value.suspendedState, 'Suspending'); assert.equal(f.subject.value.machineIdleState, 'Idle');
  assert.deepEqual(Object.keys(f.subject.value).sort(), Object.keys(original).sort());
});
test('disabling restores the original next descriptor and latest authored Teams state', () => {
  const f = fixture(); const originalNext = f.subject.next; f.adapter.receive(active);
  const latest = state({windowState: 'InFocus', machineLockState: 'Unknown'}); f.subject.next(latest);
  assert.equal(f.subject.value.machineIdleState, 'Active');
  f.adapter.receive({...active, enabled: false});
  assert.equal(f.subject.next, originalNext); assert.equal(Object.hasOwn(f.subject, 'next'), false);
  assert.equal(f.subject.value, latest); assert.equal(f.adapter.getStats().hooked, false);
});
test('frozen providers and unsupported state shapes fail closed', () => {
  const frozen = fixture(); Object.freeze(frozen.subject); frozen.adapter.receive(active);
  assert.equal(frozen.adapter.getStats().hooked, false);
  const changed = fixture({...state(), privateMessage: 'must not be read'}); changed.adapter.receive(active);
  assert.equal(changed.adapter.getStats().hooked, false);
  const future = fixture(); future.adapter.receive({...active, locked: 'not boolean'}); assert.equal(future.scans(), 0);
});
test('a replaced method is respected and unknown future updates pass through unmodified', () => {
  const f = fixture(); f.adapter.receive(active);
  const replacement = function (value) {this.value = value;}; f.subject.next = replacement;
  f.adapter.receive({...active, locked: true}); assert.equal(f.subject.next, replacement); assert.equal(f.adapter.getStats().hooked, false);
  const future = fixture(); const next = future.subject.next; future.adapter.receive(active);
  const value = {...state(), machineIdleState: 'FutureEnum'}; future.subject.next(value);
  assert.equal(future.subject.next, next); assert.equal(future.subject.value, value); assert.equal(future.adapter.getStats().unsupported, true);
});
test('disposed or throwing provider state detaches safely instead of breaking the page', () => {
  const f = fixture(); const next = f.subject.next; f.adapter.receive(active); f.subject.isStopped = true;
  f.adapter.receive({...active, locked: true}); assert.equal(f.adapter.getStats().hooked, false); assert.equal(f.subject.next, next);
  const changed = fixture(); changed.adapter.receive(active); changed.subject.unavailable = true;
  changed.adapter.receive({...active, locked: true});
  assert.equal(changed.adapter.getStats().hooked, false); assert.equal(changed.adapter.getStats().unsupported, true);
});
test('bounded discovery uses only known provider fields, never message or getter traversal', () => {
  const f = fixture(); const fiber = {memoizedProps: {value: {coreServices: {hostState: f.host},
    get messages() {throw Error('must not inspect messages');}, get users() {throw Error('must not inspect users');}}}};
  fiber.child = fiber;
  const document = {getElementById: () => ({'__reactContainer$test': fiber})};
  assert.equal(api.discover(document), f.host);
  assert.equal(api.discover({getElementById: () => ({get '__reactContainer$test'() {throw Error('getter must not execute');}})}), null);
});
test('browser installation is restricted to configured secure top-level Teams pages', async () => {
  for (const option of [{origin:'https://login.example.com'}, {origin:'https://teams.cloud.microsoft', frame:true}, {origin:'https://teams.cloud.microsoft', secure:false}]) {
    const calls = [];
    const window = {isSecureContext: option.secure !== false, location: {origin: option.origin}, __msteamsAllowedNotificationOrigins:['https://teams.cloud.microsoft'],
      document: {getElementById: () => {throw Error('must not inspect untrusted page');}},
      setInterval() {}, clearInterval() {}, __TAURI_INTERNALS__: {invoke: command => {calls.push(command); return Promise.resolve(active);}}};
    window.top = option.frame ? {} : window;
    vm.runInNewContext(source, {window}); await Promise.resolve();
    assert.deepEqual(calls, []); assert.equal(window.__msteamsScreenPresence, undefined);
  }
});
test('initial native status waits for committed page load and honors a disabled native payload', async () => {
  const calls = [], listeners = {};
  const window = {isSecureContext:true,location:{origin:'https://teams.cloud.microsoft'},__msteamsAllowedNotificationOrigins:['https://teams.cloud.microsoft'],
    document:{readyState:'loading',getElementById:()=>{throw Error('disabled bridge must not scan');}},
    setInterval(){},clearInterval(){},addEventListener:(name,fn)=>{listeners[name]=fn;},
    __TAURI_INTERNALS__:{invoke:command=>{calls.push(command);return Promise.resolve({...active,enabled:false,known:false});}}};
  window.top=window;vm.runInNewContext(source,{window});assert.deepEqual(calls,[]);
  listeners.load();await Promise.resolve();await Promise.resolve();
  assert.deepEqual(calls,['screen_presence_status']);assert.equal(window.__msteamsScreenPresence.getStats().hooked,false);
});
test('a late initial enabled response cannot undo a newer native opt-out', async () => {
  let resolveInitial;
  const window = {isSecureContext:true,location:{origin:'https://teams.cloud.microsoft'},__msteamsAllowedNotificationOrigins:['https://teams.cloud.microsoft'],
    document:{readyState:'complete',getElementById:()=>{throw Error('opt-out must prevent discovery');}},
    setInterval(){throw Error('opt-out must not start a timer');},clearInterval(){},
    __TAURI_INTERNALS__:{invoke:()=>new Promise(resolve=>{resolveInitial=resolve;})}};
  window.top=window;vm.runInNewContext(source,{window});
  window.__msteamsScreenPresence.receive({...active,enabled:false,known:false});
  resolveInitial(active);await Promise.resolve();await Promise.resolve();
  assert.equal(window.__msteamsScreenPresence.getStats().enabled,false);
  assert.equal(window.__msteamsScreenPresence.getStats().hooked,false);
});
test('native state changes update an already-subscribed authoritative subject only when necessary', () => {
  const f = fixture(); f.adapter.receive(active); const count = f.subject.events.length;
  f.adapter.receive(active); assert.equal(f.subject.events.length, count);
  f.adapter.receive({...active, locked: true}); assert.equal(f.subject.events.length, count + 1);
  const stats = f.adapter.getStats(); assert.equal(stats.enabled, true); assert.equal(stats.native_known, true); assert.equal(stats.host_found, true); assert.equal(stats.hooked, true); assert.equal(stats.mapped_idle, true);
  assert.ok(Object.values(stats).every(value => typeof value === 'boolean' || typeof value === 'number'));
});
