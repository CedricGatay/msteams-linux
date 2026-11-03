const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const adapterSource = fs.readFileSync(path.join(__dirname, '../src-tauri/src/teams-foreground-adapter.js'), 'utf8');
const moduleContext = {module: {exports: {}}, setInterval, clearInterval};
vm.runInNewContext(adapterSource, moduleContext);
const {createAdapter} = moduleContext.module.exports;

class Handler {
  constructor(reason = 'skippedAppInFocus') { this.reason = reason; this.activations = []; this.originalCalls = []; }
  tryShowBrowserNotification(...args) { this.originalCalls.push(args); return Promise.resolve(this.reason); }
  isOutOfTabLocalSettingEnabled() { return true; }
  activateBrowserToast(options) { this.activations.push(options); return Promise.resolve(); }
}
function toast({title = 'Synthetic sender', body = 'Preview hidden', previous} = {}) {
  const options = {targetEntity: {id: 'synthetic-only'}};
  let get = () => previous;
  return {
    options,
    getWebToastOptions() { return options; },
    getNotificationOptionsPayload() { return {notificationTitle: title, notificationOptions: {body, tag: undefined}}; },
    registerGetNotificationCallback(callback) { get = callback; },
    getNotification() { return get(); },
  };
}
function fixture({enabled = true, trace = false, reason, permission = 'granted', failDelivery = false, deferred = false, handler = new Handler(reason), extraProps = {}} = {}) {
  const created = [];
  let resolve;
  const delivery = deferred ? new Promise(r => {resolve = r;}) : null;
  const root = {__msteamsForegroundBridge: {
    permission: () => permission,
    create: (title, options) => {
      const notification = new EventTarget();
      notification._closed = false;
      notification._delivery = delivery || (failDelivery ? Promise.reject(new Error('Synthetic private error')) : Promise.resolve());
      notification.close = () => { notification._closed = true; notification.dispatchEvent(new Event('close')); };
      created.push({title, options, notification});
      return notification;
    },
  }};
  const toasts = {showMessage() {return 'original-result';}};
  const fiber = {memoizedProps: {coreServices: {notificationsHandler: handler}, entityCommanding: {toasts}, ...extraProps}};
  const container = {'__reactContainer$synthetic': {stateNode: {current: fiber}}};
  const timers = new Map(); let nextTimer = 0;
  const document = {getElementById: () => container, readyState: 'complete'};
  const adapter = createAdapter({window: root, document, enabled, trace,
    setInterval: fn => {timers.set(++nextTimer, fn); return nextTimer;}, clearInterval: id => timers.delete(id)});
  return {adapter, handler, created, root, fiber, container, document, timers, toasts, resolve};
}
const settle = () => new Promise(resolve => setImmediate(resolve));
function browserEntry(f, reply) {
  f.root.top = f.root;
  f.root.addEventListener = () => {};
  f.root.__TAURI_INTERNALS__ = {invoke: command => {
    assert.equal(command, 'foreground_notifications_enabled');
    return reply;
  }};
  let id = 100;
  vm.runInNewContext(adapterSource, {window: f.root, document: f.document,
    location: {hostname: 'teams.cloud.microsoft'},
    setInterval: fn => {f.timers.set(++id, fn); return id;}, clearInterval: key => f.timers.delete(key)});
  return f.root.__msteamsForegroundAdapter;
}

test('browser entry loads current native opt-in instead of a stale initial script flag', async () => {
  const f = fixture();
  const api = browserEntry(f, Promise.resolve(true));
  await settle();
  assert.equal(api.snapshot().enabled, true);
  assert.equal(api.snapshot().hooked, true);
  assert.equal(await f.handler.tryShowBrowserNotification(toast()), 'queued');
  api.stop();
});

test('late preference response cannot undo a hot toggle', async () => {
  for (const toggle of [true, false]) {
    const f = fixture(); let resolve;
    const api = browserEntry(f, new Promise(r => {resolve = r;}));
    api.setEnabled(toggle);
    resolve(!toggle);
    await settle();
    assert.equal(api.snapshot().enabled, toggle);
    api.stop();
  }
});

test('approved focus-skipped candidate uses prepared preview and lifecycle with original receiver/arguments', async () => {
  const f = fixture();
  const candidate = toast();
  assert.equal(f.adapter.scan(), true);
  const result = await f.handler.tryShowBrowserNotification(candidate, 'original-extra-argument');
  assert.equal(result, 'queued');
  assert.deepEqual(f.handler.originalCalls, [[candidate, 'original-extra-argument']]);
  assert.equal(f.created.length, 1);
  assert.equal(f.created[0].title, 'Synthetic sender');
  assert.equal(f.created[0].options.body, 'Preview hidden');
  assert.equal(candidate.getNotification(), f.created[0].notification);
  f.created[0].notification.dispatchEvent(new Event('click'));
  await settle();
  assert.deepEqual(f.handler.activations, [candidate.options]);
  assert.equal(f.adapter.snapshot().mirrored, 1);
});

test('queued browser delivery, disabled preferences and unknown results are never overridden', async () => {
  for (const reason of ['queued', 'skippedNoPermissions', 'skippedReasonUnknown', 'future-result']) {
    const f = fixture({reason});
    f.adapter.scan();
    assert.equal(await f.handler.tryShowBrowserNotification(toast()), reason);
    assert.equal(f.created.length, 0);
  }
});

test('observing while opt-in is off never changes delivery', async () => {
  const f = fixture({enabled: false, trace: true});
  f.adapter.scan();
  assert.equal(await f.handler.tryShowBrowserNotification(toast()), 'skippedAppInFocus');
  assert.equal(f.created.length, 0);
  assert.equal(f.adapter.snapshot().skipped_focus, 1);
});

test('denied native authorization is never bypassed', async () => {
  const f = fixture({permission: 'denied'});
  f.adapter.scan();
  assert.equal(await f.handler.tryShowBrowserNotification(toast()), 'skippedAppInFocus');
  assert.equal(f.created.length, 0);
});

test('same pending toast is deduplicated without comparing private message text', async () => {
  const f = fixture({deferred: true});
  const candidate = toast();
  f.adapter.scan();
  const one = f.handler.tryShowBrowserNotification(candidate);
  const two = f.handler.tryShowBrowserNotification(candidate);
  await settle();
  assert.equal(f.created.length, 1);
  f.resolve();
  assert.deepEqual(await Promise.all([one, two]), ['queued', 'queued']);
  assert.equal(await f.handler.tryShowBrowserNotification(candidate), 'queued');
  assert.equal(f.created.length, 1);
  f.created[0].notification.close();
  assert.equal(await f.handler.tryShowBrowserNotification(candidate), 'skippedAppInFocus');
  assert.equal(f.created.length, 1);
});

test('failed native delivery restores prior callback, closes partial notification, and leaves original result', async () => {
  const previous = {synthetic: true};
  const f = fixture({failDelivery: true});
  const candidate = toast({previous});
  f.adapter.scan();
  assert.equal(await f.handler.tryShowBrowserNotification(candidate), 'skippedAppInFocus');
  assert.equal(candidate.getNotification(), previous);
  assert.equal(f.created[0].notification._closed, true);
  assert.equal(f.adapter.snapshot().mirror_errors, 1);
});

test('unsupported service/payload/candidate shape falls back without serializing arbitrary objects', async () => {
  const f = fixture(); f.adapter.scan();
  assert.equal(await f.handler.tryShowBrowserNotification({}), 'skippedAppInFocus');
  const malformed = toast({body: {private: 'must not stringify'}});
  assert.equal(await f.handler.tryShowBrowserNotification(malformed), 'skippedAppInFocus');
  assert.equal(f.created.length, 0);
  assert.ok(!JSON.stringify(f.adapter.snapshot()).includes('private'));
});

test('original promise rejection and synchronous return values are preserved', async () => {
  const error = new Error('original synthetic error');
  const rejected = new Handler();
  rejected.tryShowBrowserNotification = () => Promise.reject(error);
  const f = fixture({handler: rejected}); f.adapter.scan();
  await assert.rejects(f.handler.tryShowBrowserNotification(toast()), candidate => candidate === error);
  assert.equal(f.created.length, 0);
  const sync = new Handler();
  const value = {};
  Object.defineProperty(value, 'then', {get() {throw new Error('Must not invoke getter');}});
  sync.tryShowBrowserNotification = () => value;
  const g = fixture({handler: sync}); g.adapter.scan();
  assert.equal(g.handler.tryShowBrowserNotification(toast()), value);
});

test('disabling restores the exact original own descriptor and respects application replacements', async () => {
  const handler = new Handler();
  Object.defineProperty(handler, 'tryShowBrowserNotification', {value: handler.tryShowBrowserNotification, writable: true, configurable: true, enumerable: true});
  const descriptor = Object.getOwnPropertyDescriptor(handler, 'tryShowBrowserNotification');
  const f = fixture({handler}); f.adapter.scan();
  f.adapter.setEnabled(false);
  assert.deepEqual(Object.getOwnPropertyDescriptor(handler, 'tryShowBrowserNotification'), descriptor);
  f.adapter.setEnabled(true);
  const replacement = () => Promise.resolve('application-replacement');
  handler.tryShowBrowserNotification = replacement;
  f.adapter.stop();
  assert.equal(handler.tryShowBrowserNotification, replacement);
});

test('compatible own method flags are preserved while wrapping and restoring', () => {
  for (const flags of [{writable: true, configurable: false}, {writable: false, configurable: true}]) {
    const handler = new Handler();
    Object.defineProperty(handler, 'tryShowBrowserNotification', {value: handler.tryShowBrowserNotification, enumerable: false, ...flags});
    const original = Object.getOwnPropertyDescriptor(handler, 'tryShowBrowserNotification');
    const f = fixture({handler}); f.adapter.scan();
    assert.equal(f.adapter.snapshot().hooked, true);
    const wrapped = Object.getOwnPropertyDescriptor(handler, 'tryShowBrowserNotification');
    assert.equal(wrapped.writable, flags.writable);
    assert.equal(wrapped.configurable, flags.configurable);
    f.adapter.stop();
    assert.deepEqual(Object.getOwnPropertyDescriptor(handler, 'tryShowBrowserNotification'), original);
  }
});

test('discovery avoids getters/message fields and terminates on cyclic fibers', () => {
  const f = fixture();
  let getterCalls = 0;
  Object.defineProperty(f.fiber.memoizedProps, 'context', {get() {getterCalls++; throw new Error('Must not read');}});
  Object.defineProperty(f.fiber.memoizedProps, 'message', {get() {throw new Error('Must not read message');}});
  f.fiber.child = f.fiber;
  assert.equal(f.adapter.scan(), true);
  assert.equal(getterCalls, 0);
});

test('missing or frozen handlers fail safely and discovery is bounded', () => {
  const f = fixture();
  delete f.fiber.memoizedProps.coreServices.notificationsHandler;
  f.adapter.start();
  for (let i = 0; i < 40; i++) for (const fn of [...f.timers.values()]) fn();
  assert.equal(f.adapter.snapshot().discovery_exhausted, true);
  assert.equal(f.timers.size, 0);
  const handler = Object.preventExtensions(new Handler());
  const g = fixture({handler}); g.adapter.scan();
  assert.equal(g.adapter.snapshot().hooked, false);
});

test('service adapter integrates with the real page bridge and native IPC without a second browser notification', async () => {
  const f = fixture();
  const calls = [];
  f.root.top = f.root;
  f.root.addEventListener = () => {};
  f.root.crypto = {randomUUID: () => 'integration'};
  f.root.__msteamsNotificationTraceEnabled = true;
  f.root.__TAURI_INTERNALS__ = {invoke: async (command, args) => {
    calls.push({command, args});
    if (command === 'notification_permission') return 'granted';
    if (command === 'foreground_notifications_enabled') return true;
  }};
  Object.assign(f.document, {hidden: false, title: 'Microsoft Teams', head: {}, documentElement: {},
    hasFocus: () => true, querySelectorAll: () => [], addEventListener() {}});
  let id = 200;
  const context = vm.createContext({window: f.root, document: f.document, location: {hostname: 'teams.cloud.microsoft'},
    Event, EventTarget, console: {warn() {}}, MutationObserver: class {observe() {}},
    setInterval: fn => {f.timers.set(++id, fn); return id;}, clearInterval: key => f.timers.delete(key), setTimeout() {}});
  const bridge = fs.readFileSync(path.join(__dirname, '../src-tauri/src/notification-bridge.js'), 'utf8');
  vm.runInContext(bridge, context);
  vm.runInContext(adapterSource, context);
  await settle();
  const candidate = toast();
  assert.equal(await f.handler.tryShowBrowserNotification(candidate), 'queued');
  const deliveries = calls.filter(call => call.command === 'show_notification');
  assert.equal(deliveries.length, 1);
  assert.equal(deliveries[0].args.body, 'Preview hidden');
  const snapshot = f.root.__msteamsNotificationTraceSnapshot();
  assert.equal(snapshot.teams_foreground_calls, 1);
  assert.equal(snapshot.window_calls, 0);
  assert.equal(snapshot.teams_adapter.mirrored, 1);
  f.root.__msteamsNotificationClick(deliveries[0].args.id);
  await settle();
  assert.equal(f.handler.activations.length, 1);
  // Teams' in-app timeout must not cancel native presentation or activation.
  candidate.getNotification().close();
  assert.equal(calls.filter(call => call.command === 'close_notification').length, 0);
  assert.equal(await f.handler.tryShowBrowserNotification(candidate), 'queued');
  assert.equal(calls.filter(call => call.command === 'show_notification').length, 1);
  f.root.__msteamsNotificationClick(deliveries[0].args.id);
  await settle();
  assert.equal(f.handler.activations.length, 2);
  assert.equal(f.root.__msteamsNotificationTraceSnapshot().retained_after_close, 1);
  f.root.__msteamsForegroundAdapter.stop();
});

test('trace observes only in-app command counts and known synthetic target, preserving return', () => {
  const f = fixture({trace: true}); f.adapter.scan();
  assert.equal(f.toasts.showMessage('testNotification', {private: 'synthetic body'}), 'original-result');
  f.toasts.showMessage('private-conversation-id', {private: 'another body'});
  const snapshot = f.adapter.snapshot();
  assert.equal(snapshot.in_app_calls, 2);
  assert.equal(snapshot.in_app_tests, 1);
  assert.ok(!JSON.stringify(snapshot).includes('private'));
  assert.ok(!JSON.stringify(snapshot).includes('body'));
});
