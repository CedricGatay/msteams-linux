const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/zoom-hotkeys.js'), 'utf8');
function fixture({origin = 'https://teams.cloud.microsoft', frame = false} = {}) {
  const listeners = [], calls = [];
  const window = {__msteamsAllowedNotificationOrigins: ['https://teams.cloud.microsoft'],
    __TAURI_INTERNALS__: {invoke: (command, args) => {calls.push({command, args}); return Promise.resolve();}}};
  window.top = frame ? {} : window;
  const context = vm.createContext({window, location: {origin},
    document: {addEventListener: (name, handler) => listeners.push({name, handler})}, console: {warn() {}}});
  vm.runInContext(source, context);
  return {listeners, calls, context};
}
function press(f, overrides = {}) {
  const event = {metaKey: true, ctrlKey: false, altKey: false, isComposing: false, defaultPrevented: false,
    preventDefault() {this.prevented = true;}, stopPropagation() {this.stopped = true;}, ...overrides};
  f.listeners[0].handler(event); return event;
}
test('supports logical plus/equal/minus/reset including shifted plus', () => {
  const f = fixture();
  for (const [key, action] of [['+', 'in'], ['=', 'in'], ['-', 'out'], ['0', 'reset']]) {
    const event = press(f, {key, shiftKey: key === '+'});
    assert.equal(event.prevented, true); assert.equal(event.stopped, true);
    assert.equal(f.calls.at(-1).command, 'zoom_teams'); assert.equal(f.calls.at(-1).args.action, action);
  }
});
test('does not intercept typing, other shortcuts, composition or handled keys', () => {
  const f = fixture();
  for (const overrides of [{key: '+', metaKey: false}, {key: '+', ctrlKey: true}, {key: '+', altKey: true},
    {key: '+', isComposing: true}, {key: '+', defaultPrevented: true}, {key: 'c'}]) {
    assert.equal(press(f, overrides).prevented, undefined);
  }
  assert.equal(f.calls.length, 0);
});
test('does not install into auth/unconfigured pages or subframes; injection is idempotent', () => {
  assert.equal(fixture({origin: 'https://login.example.com'}).listeners.length, 0);
  assert.equal(fixture({frame: true}).listeners.length, 0);
  const f = fixture(); vm.runInContext(source, f.context);
  assert.equal(f.listeners.length, 1);
});
