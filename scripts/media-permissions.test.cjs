const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/media-permissions.js'), 'utf8');

class Element {
  constructor(tag) {this.tag = tag; this.children = []; this.attributes = {}; this.style = {}; this.listeners = {}; this.parentNode = null; this.textContent = '';}
  appendChild(node) {this.children.push(node); node.parentNode = this; return node;}
  remove() {if (this.parentNode) this.parentNode.children = this.parentNode.children.filter(node => node !== this); this.parentNode = null;}
  attachShadow() {this.shadowForTest = new Element('shadow'); return this.shadowForTest;}
  setAttribute(name, value) {this.attributes[name] = value;}
  getAttribute(name) {return this.attributes[name] || null;}
  addEventListener(name, callback) {this.listeners[name] = callback;}
  get childElementCount() {return this.children.length;}
  getClientRects() {return this.getAttribute('aria-hidden') === 'true' ? [] : [{}];}
  click() {return this.listeners.click?.({isTrusted: true, stopPropagation() {}, preventDefault() {}});}
  set innerHTML(_) {throw Error('must not inject HTML strings');}
}
function descendants(element) {
  if (!element) return [];
  return [element, ...(element.shadowForTest ? descendants(element.shadowForTest) : []), ...element.children.flatMap(descendants)];
}
function fixture({host = 'teams.cloud.microsoft', frame = false, denyAudio = false, ready = true, shell = true,
  busy = false, permissions = {microphone: 'prompt', camera: 'prompt'}, native = null,
  quickTimeout = false, watchTimeout = false} = {}) {
  const calls = [], stopped = [], queries = [], nativeCalls = [], listeners = {}, observers = [], docListeners = {};
  const body = new Element('body'), app = new Element('app'), main = new Element('main'), navigation = new Element('nav');
  body.appendChild(app); app.id = 'app'; app.appendChild(main); app.appendChild(navigation);
  main.appendChild(new Element('content')); navigation.appendChild(new Element('content'));
  if (busy) main.setAttribute('aria-busy', 'true');
  const document = {body, readyState: ready ? 'complete' : 'loading', visibilityState: 'visible',
    createElement: tag => new Element(tag),
    getElementById: id => descendants(body).find(element => element.id === id) || null,
    querySelector: selector => !shell ? null : selector.includes('app-layout-area--main') ? main : navigation,
    addEventListener: (name, callback) => {docListeners[name] = callback;},
    removeEventListener: name => {delete docListeners[name];},
  };
  const window = {isSecureContext: true,
    __msteamsHasNativeMediaStatus: native !== null,
    __TAURI_INTERNALS__: {invoke: async command => {
      nativeCalls.push(command); assert.equal(command, 'media_permission_status');
      if (native instanceof Error) throw native;
      return native;
    }},
    addEventListener: (name, callback) => {listeners[name] = callback;},
  };
  window.top = frame ? {} : window;
  const context = vm.createContext({window, document,
    setTimeout: (callback, delay) => {
      const ms = delay === 750 ? 1 : delay === 120000 ? (watchTimeout ? 5 : 2000) : quickTimeout ? Math.min(delay, 5) : delay;
      const timer = setTimeout(callback, ms); if (delay === 120000) timer.unref(); return timer;
    }, clearTimeout,
    MutationObserver: class {
      constructor(callback) {this.callback = callback; this.active = false; observers.push(this);}
      observe() {this.active = true;}
      disconnect() {this.active = false;}
    },
    location: {hostname: host, origin: 'https://' + host},
    navigator: {
      permissions: permissions === null ? undefined : {query: async ({name}) => {
        queries.push(name);
        if (permissions[name] === 'unsupported') throw new TypeError('Unsupported permission name');
        if (permissions[name] === 'pending') return new Promise(() => {});
        return {state: permissions[name]};
      }},
      mediaDevices: {getUserMedia: async constraints => {
        const kind = Object.keys(constraints)[0]; calls.push(kind);
        if (denyAudio && kind === 'audio') throw Object.assign(new Error(), {name: 'NotAllowedError'});
        const name = kind === 'audio' ? 'microphone' : 'camera';
        if (permissions && !['pending', 'unsupported'].includes(permissions[name])) permissions[name] = 'granted';
        if (native && !(native instanceof Error)) native[kind] = 'authorized';
        return {getTracks: () => [{stop: () => stopped.push(kind)}]};
      }},
    },
  });
  function notify() {for (const observer of observers) if (observer.active) observer.callback([]);}
  return {context, window, document, calls, stopped, listeners, queries, nativeCalls, permissions, native, main, observers,
    setShell: value => {shell = value; notify();}, notify,
    load: () => {document.readyState = 'complete'; listeners.load?.({type: 'load'});},
    prompt: () => body.children.find(node => node.id === '__msteams-media-authorization'),
    buttons: () => descendants(body).filter(node => node.tag === 'button'),
    message: () => descendants(body).find(node => node.getAttribute('role') === 'status')?.textContent,
    setDenyAudio: value => {denyAudio = value;},
  };
}
const wait = (ms = 15) => new Promise(resolve => setTimeout(resolve, ms));
function start(f) {vm.runInContext(source, f.context);}

test('no permission checks before load or while the Teams shell is still loading', async () => {
  const f = fixture({ready: false, shell: false}); start(f); await wait();
  assert.deepEqual(f.queries, []); assert.deepEqual(f.calls, []); assert.equal(f.prompt(), undefined);
  f.load(); await wait(); assert.deepEqual(f.queries, []);
  f.setShell(true); await wait();
  assert.deepEqual(f.queries, ['microphone', 'camera']); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
  assert.ok(f.observers.every(observer => !observer.active));
});
test('busy or background shell does not show the bar until ready/visible', async () => {
  const f = fixture({busy: true}); start(f); await wait(); assert.deepEqual(f.queries, []);
  f.main.setAttribute('aria-busy', 'false'); f.notify(); await wait(); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
  const background = fixture(); background.document.visibilityState = 'hidden'; start(background); await wait();
  assert.deepEqual(background.queries, []);
});
test('first-use permissions show a bar without capture; only Enable requests and stops tracks', async () => {
  const f = fixture(); start(f); await wait();
  assert.ok(f.prompt()); assert.deepEqual(f.calls, []); assert.equal(f.buttons()[0].textContent, 'Enable access');
  await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video']);
  assert.deepEqual(f.stopped, ['audio', 'video']); assert.equal(f.prompt(), undefined);
});
test('already granted access produces neither a bar nor device activation', async () => {
  const f = fixture({permissions: {microphone: 'granted', camera: 'granted'}}); start(f); await wait();
  assert.deepEqual(f.calls, []); assert.equal(f.prompt(), undefined);
  assert.equal(f.window.__msteamsMediaPermissionResults.audio, 'granted');
});
test('Safari unsupported queries use native macOS status without capture or a needless bar', async () => {
  const f = fixture({permissions: {microphone: 'unsupported', camera: 'unsupported'}, native: {audio: 'authorized', video: 'authorized'}});
  start(f); await wait(); assert.deepEqual(f.nativeCalls, ['media_permission_status']);
  assert.deepEqual(f.calls, []); assert.equal(f.prompt(), undefined);
  assert.equal(f.window.__msteamsMediaPermissionResults.audio, 'os-authorized');
});
test('only the missing device is requested after an explicit click', async () => {
  const f = fixture({permissions: null, native: {audio: 'authorized', video: 'not-determined'}}); start(f); await wait();
  assert.deepEqual(f.calls, []); await f.buttons()[0].click();
  assert.deepEqual(f.calls, ['video']); assert.deepEqual(f.stopped, ['video']);
});
test('site permission alone does not stand in for OS authorization, but still never auto-captures', async () => {
  const f = fixture({permissions: {microphone: 'granted', camera: 'granted'}, native: {audio: 'not-determined', video: 'not-determined'}});
  start(f); await wait(); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
  await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video']);
});
test('denied/restricted access offers settings guidance and Check again, not repeated capture', async () => {
  const f = fixture({permissions: null, native: {audio: 'denied', video: 'restricted'}}); start(f); await wait();
  assert.match(f.message(), /System Settings/); assert.equal(f.buttons()[0].textContent, 'Check again');
  await f.buttons()[0].click(); assert.deepEqual(f.calls, []);
  f.native.audio = f.native.video = 'authorized'; await f.buttons()[0].click(); assert.equal(f.prompt(), undefined);
});
test('unknown/unavailable status is never probed through hardware without a button click', async () => {
  for (const options of [{permissions: null}, {permissions: null, native: new Error('unavailable')},
    {native: {audio: 'unexpected', video: 'unexpected'}}]) {
    const f = fixture(options); start(f); await wait(); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
    await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video']);
  }
});
test('Not now dismisses; Help reopens the bar without authorizing', async () => {
  const f = fixture(); start(f); await wait(); await f.buttons()[1].click();
  assert.equal(f.prompt(), undefined); assert.deepEqual(f.calls, []);
  await f.window.__msteamsRequestMediaPermissions(); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
});
test('synthetic clicks and dismissing during a pending check never authorize devices', async () => {
  const f = fixture({permissions: {microphone: 'pending', camera: 'pending'}, quickTimeout: true}); start(f); await wait(25);
  await f.buttons()[0].listeners.click({isTrusted: false, stopPropagation() {}}); assert.deepEqual(f.calls, []);
  const pending = f.buttons()[0].click(); await f.buttons()[1].click(); await pending;
  assert.deepEqual(f.calls, []); assert.equal(f.prompt(), undefined);
});
test('Help can check an already-authorized app without opening devices', async () => {
  const f = fixture({permissions: {microphone: 'granted', camera: 'granted'}}); start(f); await wait();
  await f.window.__msteamsRequestMediaPermissions(); assert.match(f.message(), /already authorized/);
  await f.buttons()[0].click(); assert.deepEqual(f.calls, []); assert.equal(f.prompt(), undefined);
});
test('Check again never turns into authorization if permission was changed since the bar appeared', async () => {
  const f = fixture({permissions: null, native: {audio: 'authorized', video: 'authorized'}}); start(f); await wait();
  await f.window.__msteamsRequestMediaPermissions();
  f.native.video = 'not-determined'; await f.buttons()[0].click();
  assert.deepEqual(f.calls, []); assert.equal(f.buttons()[0].textContent, 'Enable access');
  await f.buttons()[0].click(); assert.deepEqual(f.calls, ['video']);
});
test('a blocked sibling is skipped while an explicitly enabled missing permission can be requested', async () => {
  const f = fixture({permissions: null, native: {audio: 'not-determined', video: 'denied'}}); start(f); await wait();
  assert.equal(f.buttons()[0].textContent, 'Enable access'); await f.buttons()[0].click();
  assert.deepEqual(f.calls, ['audio']); assert.equal(f.buttons()[0].textContent, 'Check again');
});
test('microphone denial does not prevent the separately requested camera', async () => {
  const f = fixture({denyAudio: true}); start(f); await wait(); await f.buttons()[0].click();
  assert.deepEqual(f.calls, ['audio', 'video']); assert.deepEqual(f.stopped, ['video']);
  assert.equal(f.window.__msteamsMediaPermissionResults.audio, 'NotAllowedError'); assert.ok(f.prompt());
  f.setDenyAudio(false); await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video']);
  assert.equal(f.buttons()[0].textContent, 'Enable access');
  await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video', 'audio']);
});
test('repeated injection/checks and double clicks do not duplicate UI or requests', async () => {
  const f = fixture(); start(f); start(f); await wait();
  assert.equal(f.buttons().length, 2);
  const button = f.buttons()[0]; await Promise.all([button.click(), button.click()]);
  assert.deepEqual(f.calls, ['audio', 'video']);
});
test('hanging state queries are bounded and the explicit button can still request access', async () => {
  const f = fixture({permissions: {microphone: 'pending', camera: 'pending'}, native: new Error('unavailable'), quickTimeout: true});
  start(f); await wait(25); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
  await f.buttons()[0].click(); assert.deepEqual(f.calls, ['audio', 'video']);
});
test('a future/unrecognized layout times out safely; Help remains a fallback without a restart', async () => {
  const f = fixture({shell: false, watchTimeout: true}); start(f); await wait();
  assert.deepEqual(f.queries, []); assert.ok(f.observers.every(observer => !observer.active));
  await f.window.__msteamsRequestMediaPermissions(); assert.ok(f.prompt()); assert.deepEqual(f.calls, []);
});
test('SSO, subframes and insecure contexts never get checks, a bar, or capture', async () => {
  for (const options of [{host: 'login.example.com'}, {host: 'teams.cloud.microsoft.evil.com'}, {frame: true}]) {
    const f = fixture(options); start(f); await wait();
    assert.deepEqual(f.calls, []); assert.deepEqual(f.queries, []); assert.deepEqual(f.nativeCalls, []); assert.equal(f.prompt(), undefined);
  }
  const f = fixture(); f.window.isSecureContext = false; start(f); await wait(); assert.deepEqual(f.queries, []);
});
test('the consent bar is non-modal, has accessible controls, and never steals focus', async () => {
  const f = fixture(); start(f); await wait();
  const region = descendants(f.prompt()).find(node => node.getAttribute('role') === 'region');
  assert.ok(region); assert.equal(region.getAttribute('aria-label'), 'Teams Shell media permissions');
  assert.equal(region.getAttribute('aria-modal'), null); assert.equal(f.buttons()[1].textContent, 'Not now');
  assert.doesNotMatch(source, /\.focus\(/);
});
test('native status is read-only and stays limited to the main Teams origins', () => {
  const capability = JSON.parse(fs.readFileSync(path.join(__dirname, '../src-tauri/capabilities/notification-bridge.json'), 'utf8'));
  assert.deepEqual(capability.windows, ['main']); assert.ok(capability.permissions.includes('allow-media-permission-status'));
  assert.deepEqual(capability.remote.urls, ['https://teams.cloud.microsoft/*', 'https://teams.microsoft.com/*']);
  const native = fs.readFileSync(path.join(__dirname, '../src-tauri/src/media_permissions.rs'), 'utf8');
  assert.match(native, /authorizationStatusForMediaType/); assert.doesNotMatch(native, /requestAccessForMediaType|AVCaptureSession|devicesWithMediaType/);
});
