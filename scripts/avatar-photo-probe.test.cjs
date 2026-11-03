const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const snapshotSource = fs.readFileSync(path.join(__dirname, '../src-tauri/src/avatar-diagnostics.js'), 'utf8');
const template = fs.readFileSync(path.join(__dirname, 'avatar-photo-probe.js'), 'utf8');
const source = template.replace('__MSTEAMS_SNAPSHOT_SOURCE__', () => snapshotSource);
function fixture(url, {status = 403, statuses, rejection = false, avatarProps = {}, avatarElements = [], scoped = false, refresh} = {}) {
  const calls = [];
  const store = {get: key => key === 'baseImageUrl' ? 'https://photos.example.com/api/' : true};
  const fiber = {memoizedProps: {avatarUrl: url, ...avatarProps, coreServices: {
    stateManagementServices: {userWindowStore: store}, mediaAuthService: {
      _isInitialized: true, _baseImageUrlSubject: {_value: 'https://photos.example.com/api/'},
      _isImageAuthCookieValidSubject: {_value: true}, _isMTMAProfilePicturesEnabled: scoped,
      _userObjectId: 'private-current-account', setupImageAuthCookieAndExpiry: refresh
    }
  }}};
  const window = {__msteamsAllowedNotificationOrigins: ['https://teams.cloud.microsoft'], fetch: async (...args) => {
    calls.push(args);
    if (rejection) throw Error('private-error-token');
    const responseStatus = statuses ? statuses[Math.min(calls.length - 1, statuses.length - 1)] : status;
    return {status: responseStatus, get headers() {throw Error('must not inspect headers');}, body: {cancel: async () => {}}};
  }};
  window.top = window;
  const context = vm.createContext({window, URL, navigator: {}, location: {origin: 'https://teams.cloud.microsoft', href: 'https://teams.cloud.microsoft/v2'},
    document: {querySelectorAll: () => avatarElements, getElementById: () => ({'__reactContainer$test': fiber})}, performance: {getEntriesByType: () => []}});
  return {context, window, calls, run: () => vm.runInContext(source, context)};
}
test('one existing HTTPS photo request uses standard credentials/CORS and records only HTTP status', async () => {
  const f = fixture('https://photos.example.com/api/beta/users/private-person/profilepicturev2?displayName=private-name');
  assert.equal(f.run().photo_requests, 1);
  await new Promise(resolve => setImmediate(resolve));
  const result = f.run();
  assert.equal(f.calls.length, 1); assert.equal(result.photo_http_403, 1);
  assert.equal(f.calls[0][1].credentials, 'include'); assert.equal(f.calls[0][1].mode, 'cors');
  assert.equal(f.calls[0][1].cache, 'no-store'); assert.equal(f.calls[0][1].redirect, 'error');
  assert.deepEqual(Object.keys(f.window.__msteamsPhotoRequestProbe).sort(), ['refreshes', 'requests', 'statuses', 'version']);
  assert.doesNotMatch(JSON.stringify(result), /private|photos\.example|displayName/);
});
test('can use Teams own User generator for an existing UI entity, without mutating auth state', async () => {
  const entity = {id: 'private-id', get displayName() {throw Error('probe must not inspect names');}};
  let generated = 0;
  const generator = {profilePictureType: 'User', generateUrl(value, base, useObjectId, currentUser, size) {
    generated++; assert.equal(value, entity); assert.equal(useObjectId, false); assert.equal(size, 'HR64x64');
    return base + 'beta/users/private-id/profilepicturev2';
  }};
  const f = fixture('data:image/svg+xml,private', {avatarProps: {entity, generator}});
  assert.equal(f.run().photo_requests, 1);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(generated, 1); assert.equal(f.run().photo_http_403, 1); assert.equal(f.calls.length, 1);
});
test('finds photo generator directly above a displayed avatar even outside the root scan branch', async () => {
  const generator = {profilePictureType: 'User', generateUrl(entity, base) {return base + 'beta/users/private-id/profilepicturev2';}};
  const parent = {memoizedProps: {generator, entity: {id: 'private-id'}}};
  const element = {currentSrc: 'data:image/svg+xml,private', naturalWidth: 32, naturalHeight: 32,
    complete: true, getAttribute: () => null, closest: () => ({}),
    '__reactFiber$test': {return: parent}};
  const f = fixture('data:image/svg+xml,private', {avatarElements: [element]});
  assert.equal(f.run().photo_requests, 1);
  await new Promise(resolve => setImmediate(resolve)); assert.equal(f.calls.length, 1);
  assert.equal(f.run().photo_http_403, 1);
});
test('uses the live account-scoped generator arguments instead of an unscoped legacy URL', async () => {
  const generator = {profilePictureType: 'User', generateUrl(entity, base, useObjectId, currentAccount) {
    assert.equal(useObjectId, true); assert.equal(currentAccount, 'private-current-account');
    return base + 'beta/users/' + currentAccount + '/profilepicturev2/private-person';
  }};
  const f = fixture('data:image/svg+xml,private', {scoped: true, avatarProps: {generator, entity: {id: 'private-person'}}});
  f.run(); await new Promise(resolve => setImmediate(resolve)); assert.equal(f.calls.length, 1);
  assert.match(f.calls[0][0], /beta\/users\/private-current-account\/profilepicturev2\//);
});
test('a scoped 401 permits one official service refresh and verifies with a second GET', async () => {
  let refreshes = 0;
  const f = fixture('https://photos.example.com/api/profilepicturev2', {statuses: [401, 200], refresh(caller) {
    assert.equal(caller, 'avatar-cookie-recovery'); refreshes++; return Promise.resolve();
  }});
  f.run(); await new Promise(resolve => setImmediate(resolve));
  const result = f.run(); assert.equal(refreshes, 1); assert.equal(f.calls.length, 2);
  assert.equal(result.photo_requests, 2); assert.equal(result.photo_http_401, 1); assert.equal(result.photo_http_success, 1);
  assert.equal(result.photo_http_unknown, 0); f.run(); assert.equal(f.calls.length, 2);
});
test('does not refresh on policy denial or loop when refreshing does not repair authentication', async () => {
  let refreshes = 0;
  const denied = fixture('https://photos.example.com/api/profilepicturev2', {status: 403, refresh() {refreshes++;}});
  denied.run(); await new Promise(resolve => setImmediate(resolve)); assert.equal(refreshes, 0);
  const rejected = fixture('https://photos.example.com/api/profilepicturev2', {status: 401, refresh() {refreshes++; return Promise.resolve();}});
  rejected.run(); await new Promise(resolve => setImmediate(resolve)); const result = rejected.run();
  assert.equal(refreshes, 1); assert.equal(rejected.calls.length, 2); assert.equal(result.photo_http_401, 2);
});
test('refuses other origins, other provider paths, HTTP and default-image URLs', async () => {
  for (const url of ['https://unrelated.example.com/api/profilepicturev2', 'https://photos.example.com/elsewhere/profilepicturev2',
    'http://photos.example.com/api/profilepicturev2', 'data:image/svg+xml,private', 'https://photos.example.com/api/defaultProfilePictureV2']) {
    const f = fixture(url); assert.equal(f.run().photo_requests, 0);
    await new Promise(resolve => setImmediate(resolve)); assert.equal(f.calls.length, 0);
  }
});
test('CORS/network rejection is unknown, never claimed to be an authentication failure', async () => {
  const f = fixture('https://photos.example.com/api/profilepicturev2', {rejection: true});
  f.run(); await new Promise(resolve => setImmediate(resolve));
  const result = f.run(); assert.equal(result.photo_http_unknown, 1); assert.equal(result.photo_http_401, 0);
  assert.doesNotMatch(JSON.stringify(result), /private-error/);
});
