const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/notification-bridge.js'), 'utf8');

function fixture({permission = 'granted', requestedPermission = 'granted', failDelivery = false, failPermission = false, host = 'teams.cloud.microsoft', frame = false, loaded = true, traceEnabled = false, workerControlled = false, deliveryDeferred = null} = {}) {
  const calls = [], warnings = [], listeners = {}, intervals = [], observers = [], docListeners = {};
  const status = {permission, requestedPermission, focused: true};
  const window = {crypto: {randomUUID: () => 'fixture'}, addEventListener: (name, fn) => {listeners[name] = fn;}};
  window.top = frame ? {} : window;
  window.__msteamsNotificationTraceEnabled = traceEnabled;
  window.__TAURI_INTERNALS__ = {invoke: async (command, args) => {
    calls.push({command, args});
    if (command === 'notification_permission') {
      if (failPermission) throw new Error('Synthetic private permission error');
      return args.request ? status.requestedPermission : status.permission;
    }
    if (command === 'show_notification' && failDelivery) throw new Error('delivery failed');
    if (command === 'show_notification' && deliveryDeferred) return deliveryDeferred;
  }};
  const document = {title: 'Microsoft Teams', hidden: false, readyState: loaded ? 'complete' : 'loading',
    head: {}, documentElement: {}, hasFocus: () => status.focused, querySelectorAll: () => [],
    addEventListener: (name, fn) => {docListeners[name] = fn;}};
  class ServiceWorkerRegistration {}
  const context = vm.createContext({window, document, location: {hostname: host}, Event, EventTarget,
    navigator: {serviceWorker: {controller: workerControlled ? {} : null}},
    ServiceWorkerRegistration, MutationObserver: class {constructor(callback) {observers.push(callback);} observe() {}},
    console: {warn: (...args) => warnings.push(args)},
    setInterval: fn => intervals.push(fn), setTimeout() {},
  });
  vm.runInContext(source, context);
  return {window, document, calls, warnings, listeners, intervals, ServiceWorkerRegistration, context, status, observers, docListeners};
}
const settle = () => new Promise(resolve => setImmediate(resolve));
const callsFor = (f, name) => f.calls.filter(call => call.command === name);

test('bridges title/body/silent and emits show and unread count only after acceptance', async () => {
  const f = fixture();
  const notification = new f.window.Notification('A message', {body: 'Synthetic test body', silent: true});
  let shown = 0;
  notification.onshow = () => shown++;
  await notification._delivery;
  const payload = callsFor(f, 'show_notification')[0].args;
  assert.equal(payload.title, 'A message');
  assert.equal(payload.body, 'Synthetic test body');
  assert.equal(payload.silent, true);
  assert.equal(shown, 1);
  assert.equal(callsFor(f, 'set_unread_count').at(-1).args.count, 1);
});

test('waits for page load before querying native permission', async () => {
  const f = fixture({loaded: false});
  await settle();
  assert.equal(callsFor(f, 'notification_permission').length, 0);
  f.listeners.load();
  await settle();
  assert.equal(f.window.Notification.permission, 'granted');
});

test('does not report granted before native authorization completes', async () => {
  const f = fixture({permission: 'default'});
  assert.equal(f.window.Notification.permission, 'default');
  await settle();
  assert.equal(f.window.Notification.permission, 'granted');
  assert.deepEqual(callsFor(f, 'notification_permission').map(c => c.args.request), [false, true]);
});

test('reflects denied permission and does not deliver or increment unread', async () => {
  const f = fixture({permission: 'denied'});
  const notification = new f.window.Notification('Synthetic secret title');
  let errors = 0;
  notification.onerror = () => errors++;
  await assert.rejects(notification._delivery);
  assert.equal(f.window.Notification.permission, 'denied');
  assert.equal(callsFor(f, 'show_notification').length, 0);
  assert.equal(callsFor(f, 'set_unread_count').length, 0);
  assert.equal(errors, 1);
  assert.ok(!JSON.stringify(f.warnings).includes('secret title'));
});

test('supports legacy requestPermission callback without faking a grant', async () => {
  const f = fixture({requestedPermission: 'denied'});
  await settle();
  let callback;
  const result = await f.window.Notification.requestPermission(value => {callback = value;});
  assert.equal(result, 'denied');
  assert.equal(callback, 'denied');
  assert.equal(f.window.Notification.permission, 'denied');
});

test('native click invokes both onclick and EventTarget listeners', async () => {
  const f = fixture();
  const notification = new f.window.Notification('Synthetic test');
  let propertyClicks = 0, listenerClicks = 0;
  notification.onclick = event => {assert.equal(event.type, 'click'); propertyClicks++;};
  notification.addEventListener('click', () => listenerClicks++);
  await notification._delivery;
  f.window.__msteamsNotificationClick(callsFor(f, 'show_notification')[0].args.id);
  assert.equal(propertyClicks, 1);
  assert.equal(listenerClicks, 1);
});

test('close removes the native notification and prevents further clicks', async () => {
  const f = fixture();
  const notification = new f.window.Notification('Synthetic test');
  await notification._delivery;
  let closed = 0, clicked = 0;
  notification.onclose = () => closed++;
  notification.onclick = () => clicked++;
  notification.close();
  notification.close();
  f.window.__msteamsNotificationClick(notification._id);
  assert.equal(closed, 1);
  assert.equal(clicked, 0);
  assert.equal(callsFor(f, 'close_notification').length, 1);
});

test('Teams foreground close is logical only: delayed native presentation and click routing survive', async () => {
  const f = fixture({traceEnabled: true});
  const n = f.window.__msteamsForegroundBridge.create('Synthetic foreground', {body: 'Synthetic preview'});
  await n._delivery;
  let closed = 0, clicked = 0;
  n.onclose = () => closed++;
  n.onclick = () => clicked++;
  n.close(); n.close();
  assert.equal(closed, 1);
  assert.equal(n._nativeRetained, true);
  assert.equal(callsFor(f, 'close_notification').length, 0);
  f.window.__msteamsNotificationClick(n._id);
  assert.equal(clicked, 1);
  assert.equal(f.window.__msteamsNotificationTraceSnapshot().retained_after_close, 1);
  n._discard(); n._discard();
  assert.equal(callsFor(f, 'close_notification').length, 1);
  assert.equal(closed, 1);
  f.window.__msteamsNotificationClick(n._id);
  assert.equal(clicked, 1);
});

test('foreground close while native acknowledgement is pending does not cancel the accepted request', async () => {
  let resolve;
  const f = fixture({deliveryDeferred: new Promise(r => {resolve = r;})});
  const n = f.window.__msteamsForegroundBridge.create('Synthetic foreground');
  await settle();
  assert.equal(callsFor(f, 'show_notification').length, 1);
  n.close();
  resolve();
  await n._delivery;
  assert.equal(callsFor(f, 'close_notification').length, 0);
  assert.equal(n._nativeRetained, true);
  n._discard();
});

test('foreground close before submission still cancels delivery', async () => {
  const f = fixture();
  const n = f.window.__msteamsForegroundBridge.create('Synthetic foreground');
  n.close();
  await n._delivery;
  assert.equal(callsFor(f, 'show_notification').length, 0);
  assert.equal(n._nativeRetained, false);
});

test('discarding an old retained tagged notification cannot delete its replacement or fire close twice', async () => {
  const f = fixture();
  const first = f.window.__msteamsForegroundBridge.create('First', {tag: 'synthetic-tag'});
  await first._delivery;
  let closed = 0;
  first.onclose = () => closed++;
  first.close();
  const next = new f.window.Notification('Replacement', {tag: 'synthetic-tag'});
  await next._delivery;
  first._discard();
  assert.equal(closed, 1);
  assert.equal(callsFor(f, 'close_notification').length, 0);
  assert.equal(first._id, next._id);
  next.close();
  assert.equal(callsFor(f, 'close_notification').length, 1);
});

test('retained foreground click callbacks remain bounded', async () => {
  const f = fixture();
  const notifications = [];
  let staleClick = 0, latestClick = 0;
  for (let i = 0; i < 257; i++) {
    const n = f.window.__msteamsForegroundBridge.create('Synthetic foreground');
    await n._delivery;
    n.close(); notifications.push(n);
  }
  notifications[0].onclick = () => staleClick++;
  notifications.at(-1).onclick = () => latestClick++;
  f.window.__msteamsNotificationClick(notifications[0]._id);
  f.window.__msteamsNotificationClick(notifications.at(-1)._id);
  assert.equal(staleClick, 0);
  assert.equal(latestClick, 1);
});

test('close before delivery cancels the queued notification', async () => {
  const f = fixture();
  const notification = new f.window.Notification('Synthetic test');
  notification.close();
  await notification._delivery;
  assert.equal(callsFor(f, 'show_notification').length, 0);
  assert.equal(callsFor(f, 'set_unread_count').length, 0);
});

test('page-side service-worker registration notifications use the same native bridge', async () => {
  const f = fixture();
  await new f.ServiceWorkerRegistration().showNotification('Worker test', {body: 'Synthetic body'});
  assert.equal(callsFor(f, 'show_notification')[0].args.title, 'Worker test');
});

test('delivery errors reject page-side worker promise and emit no unread count', async () => {
  const f = fixture({failDelivery: true});
  await assert.rejects(new f.ServiceWorkerRegistration().showNotification('Synthetic secret', {body: 'Private synthetic body'}));
  assert.equal(callsFor(f, 'set_unread_count').length, 0);
  assert.ok(!JSON.stringify(f.warnings).includes('Private synthetic body'));
});

test('tagged replacement reuses native ID and closes old JS instance without deleting the replacement', async () => {
  const f = fixture();
  const first = new f.window.Notification('First', {tag: 'same-tag'});
  await first._delivery;
  let closed = 0;
  first.onclose = () => closed++;
  const second = new f.window.Notification('Second', {tag: 'same-tag'});
  await second._delivery;
  const payloads = callsFor(f, 'show_notification').map(call => call.args);
  assert.equal(payloads[0].id, payloads[1].id);
  assert.equal(closed, 1);
  assert.equal(callsFor(f, 'close_notification').length, 0);
});

test('focus resets notification-derived unread count and preserves title-derived count', async () => {
  const f = fixture();
  await new f.window.Notification('Synthetic test')._delivery;
  f.listeners.focus();
  assert.equal(callsFor(f, 'set_unread_count').at(-1).args.count, 0);
  f.document.title = '(5) Microsoft Teams';
  f.intervals[0]();
  f.listeners.focus();
  assert.equal(callsFor(f, 'set_unread_count').at(-1).args.count, 5);
});

test('refreshes native permission when returning from macOS settings', async () => {
  const f = fixture({permission: 'denied'});
  await settle();
  assert.equal(f.window.Notification.permission, 'denied');
  f.status.permission = 'granted';
  f.listeners.focus();
  await settle();
  assert.equal(f.window.Notification.permission, 'granted');
  await new f.window.Notification('Synthetic test')._delivery;
  assert.equal(callsFor(f, 'show_notification').length, 1);
});

test('does not inject a bridge into sign-in pages, unrelated hosts, or subframes', async () => {
  for (const options of [{host: 'login.microsoftonline.com'}, {host: 'teams.cloud.microsoft.evil.com'}, {frame: true}]) {
    const f = fixture(options);
    await settle();
    assert.equal(f.window.Notification, undefined);
    assert.equal(f.calls.length, 0);
  }
});

test('trace is disabled by default and adds no DOM activity observer', async () => {
  const f = fixture();
  await settle();
  assert.equal(f.window.__msteamsNotificationTraceSnapshot(), null);
  assert.equal(f.observers.length, 1);
  assert.equal(f.docListeners.click, undefined);
});

test('trace captures permission reads before and after the async native grant', async () => {
  const f = fixture({traceEnabled: true, loaded: false});
  assert.equal(f.window.Notification.permission, 'default');
  f.listeners.load();
  await settle();
  assert.equal(f.window.Notification.permission, 'granted');
  const snapshot = f.window.__msteamsNotificationTraceSnapshot();
  assert.equal(snapshot.permission_reads, 2);
  assert.deepEqual(Array.from(snapshot.events.filter(event => event.stage === 'permission_read'), event => event.permission), ['default', 'granted']);
});

test('trace separates window/registration APIs and contains no notification contents or identifiers', async () => {
  const f = fixture({traceEnabled: true, workerControlled: true});
  await new f.window.Notification('private-title', {body: 'private-body', tag: 'private-tag', data: 'private-data'})._delivery;
  await new f.ServiceWorkerRegistration().showNotification('private-worker-title', {body: 'private-worker-body'});
  const snapshot = f.window.__msteamsNotificationTraceSnapshot();
  assert.equal(snapshot.window_calls, 1);
  assert.equal(snapshot.service_worker_page_calls, 1);
  assert.equal(snapshot.send_attempts, 2);
  assert.equal(snapshot.accepted, 2);
  assert.equal(snapshot.notification_hooked, true);
  assert.equal(snapshot.service_worker_page_hooked, true);
  assert.equal(snapshot.worker_controlled, true);
  for (const secret of ['private-title', 'private-body', 'private-tag', 'private-data', 'private-worker-title', 'private-worker-body', 'page-fixture']) {
    assert.ok(!JSON.stringify(snapshot).includes(secret));
  }
});

test('trace distinguishes permission failure from native delivery failure without error strings', async () => {
  for (const failPermission of [true, false]) {
    const f = fixture({traceEnabled: true, failPermission, failDelivery: !failPermission});
    await assert.rejects(new f.window.Notification('private-failed-title')._delivery);
    const snapshot = f.window.__msteamsNotificationTraceSnapshot();
    assert.equal(snapshot.failed, 1);
    assert.equal(snapshot.accepted, 0);
    assert.equal(snapshot.send_attempts, failPermission ? 0 : 1);
    assert.equal(snapshot.permission_errors, failPermission ? 1 : 0);
    assert.ok(!JSON.stringify(snapshot).includes('private'));
    assert.ok(!JSON.stringify(f.warnings).includes('private'));
  }
});

test('trace counts in-app role activity/buttons and focus without reading DOM text', async () => {
  const f = fixture({traceEnabled: true});
  await settle();
  const target = {closest: () => ({})};
  Object.defineProperty(target, 'textContent', {get() {throw new Error('DOM text must not be read');}});
  f.status.focused = false;
  f.docListeners.click({target});
  f.observers.find(callback => callback.length === 1)([{target, addedNodes: []}]);
  const snapshot = f.window.__msteamsNotificationTraceSnapshot();
  assert.equal(snapshot.button_clicks, 1);
  assert.equal(snapshot.dom_alert_batches, 1);
  assert.equal(snapshot.window_calls, 0);
  assert.equal(snapshot.send_attempts, 0);
  assert.equal(snapshot.focused, false);
  assert.equal(snapshot.visible, true);
});

test('trace detects replaced hooks and bounds event history', async () => {
  const f = fixture({traceEnabled: true});
  await settle();
  for (let i = 0; i < 90; i++) f.docListeners.click({target: {closest: () => ({})}});
  f.window.Notification = function ReplacedNotification() {};
  const snapshot = f.window.__msteamsNotificationTraceSnapshot();
  assert.equal(snapshot.notification_hooked, false);
  assert.equal(snapshot.events.length, 64);
  assert.equal(snapshot.button_clicks, 90);
  assert.ok(!JSON.stringify(snapshot).includes('ReplacedNotification'));
});

test('notification capability is scoped to main Teams origins', () => {
  const capability = JSON.parse(fs.readFileSync(path.join(__dirname, '../src-tauri/capabilities/notification-bridge.json'), 'utf8'));
  assert.deepEqual(capability.windows, ['main']);
  assert.deepEqual(capability.remote.urls, ['https://teams.cloud.microsoft/*', 'https://teams.microsoft.com/*']);
});
