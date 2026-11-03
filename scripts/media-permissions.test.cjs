const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src-tauri/src/media-permissions.js'), 'utf8');

function fixture({host = 'teams.cloud.microsoft', frame = false, denyAudio = false, ready = true} = {}) {
  const calls = [];
  const stopped = [];
  const listeners = {};
  const window = {
    isSecureContext: true,
    addEventListener: (event, callback) => { listeners[event] = callback; },
  };
  window.top = frame ? {} : window;
  const context = vm.createContext({
    window,
    location: {hostname: host},
    document: {readyState: ready ? 'complete' : 'loading'},
    navigator: {mediaDevices: {getUserMedia: async constraints => {
      const kind = Object.keys(constraints)[0];
      calls.push(kind);
      if (denyAudio && kind === 'audio') throw Object.assign(new Error(), {name: 'NotAllowedError'});
      return {getTracks: () => [{stop: () => stopped.push(kind)}]};
    }}},
  });
  return {context, window, calls, stopped, listeners};
}
const settle = () => new Promise(resolve => setImmediate(resolve));

test('requests microphone and camera separately and immediately stops tracks', async () => {
  const f = fixture();
  vm.runInContext(source, f.context);
  await settle();
  assert.deepEqual(f.calls, ['audio', 'video']);
  assert.deepEqual(f.stopped, ['audio', 'video']);
});

test('does not request access on SSO hosts or in subframes', async () => {
  for (const options of [{host: 'login.example.com'}, {host: 'teams.cloud.microsoft.evil.com'}, {frame: true}]) {
    const f = fixture(options);
    vm.runInContext(source, f.context);
    await settle();
    assert.deepEqual(f.calls, []);
  }
});

test('microphone denial does not prevent requesting camera access', async () => {
  const f = fixture({denyAudio: true});
  vm.runInContext(source, f.context);
  await settle();
  assert.deepEqual(f.calls, ['audio', 'video']);
  assert.deepEqual(f.stopped, ['video']);
  assert.equal(f.window.__msteamsMediaPermissionResults.audio, 'NotAllowedError');
});

test('does not duplicate automatic requests within a document; requests again after navigation', async () => {
  const f = fixture();
  vm.runInContext(source, f.context);
  vm.runInContext(source, f.context);
  await settle();
  assert.deepEqual(f.calls, ['audio', 'video']);
  const reloaded = fixture();
  vm.runInContext(source, reloaded.context);
  await settle();
  assert.deepEqual(reloaded.calls, ['audio', 'video']);
});

test('allows an explicit retry after denial without reinjecting or reloading', async () => {
  const f = fixture({denyAudio: true});
  vm.runInContext(source, f.context);
  await settle();
  await f.window.__msteamsRequestMediaPermissions();
  assert.deepEqual(f.calls, ['audio', 'video', 'audio', 'video']);
});

test('does not start overlapping requests while a permission prompt is pending', async () => {
  const f = fixture();
  vm.runInContext(source, f.context);
  f.window.__msteamsRequestMediaPermissions();
  await settle();
  assert.deepEqual(f.calls, ['audio', 'video']);
});

test('waits for the Teams page to load', async () => {
  const f = fixture({ready: false});
  vm.runInContext(source, f.context);
  assert.deepEqual(f.calls, []);
  f.listeners.load();
  await settle();
  assert.deepEqual(f.calls, ['audio', 'video']);
});
