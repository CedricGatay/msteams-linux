const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/avatar-diagnostics.js'), 'utf8');
function image(src, {avatar = true, complete = true, width = 32} = {}) {
  return {currentSrc: src, complete, naturalWidth: width, naturalHeight: width,
    getBoundingClientRect() {return {width: 32, height: 32, top: 0, left: 0, right: 32, bottom: 32};},
    getAttribute(name) {assert.equal(name, 'src'); return src;},
    closest() {return avatar ? {} : null;},
    get alt() {throw Error('must not inspect personal names');}};
}
function snapshot({images = [], entries = [], origin = 'https://teams.cloud.microsoft', frame = false, timing = true, reactRoot = null} = {}) {
  const window = {__msteamsAllowedNotificationOrigins: ['https://teams.cloud.microsoft'], createImageBitmap() {},
    innerWidth: 800, innerHeight: 600, getComputedStyle: node => node.styleStub || {display: 'block', visibility: 'visible', opacity: '1'}};
  window.top = frame ? {} : window;
  return vm.runInNewContext(source, {window, navigator: {serviceWorker: {controller: {}}},
    location: {origin, href: `${origin}/v2`}, URL,
    document: {querySelectorAll: selector => selector === 'img' ? images : [{}, {}], getElementById: () => reactRoot},
    performance: {getEntriesByType() {if (!timing) throw Error('unavailable'); return entries;}}});
}
test('distinguishes avatar load failures, pending images, placeholders and unrelated images', () => {
  const result = snapshot({images: [image('https://example.com/private-photo'),
    image('blob:https://teams.cloud.microsoft/private-id', {width: 0}),
    image('https://example.com/private-avatar', {width: 0}),
    image('data:image/png;base64,private-pixels', {width: 0}),
    image('https://example.com/pending', {complete: false, width: 0}),
    image('https://example.com/unrelated', {avatar: false, width: 0}), image('', {width: 0})]});
  assert.equal(result.image_scanned, 6); assert.equal(result.image_failed, 4);
  assert.equal(result.avatar_images, 5); assert.equal(result.avatar_loaded, 1);
  assert.equal(result.avatar_failed, 3); assert.equal(result.avatar_pending, 1);
  assert.equal(result.failed_avatar_blob, 1); assert.equal(result.failed_avatar_https, 1);
  assert.equal(result.failed_avatar_data, 1);
  assert.ok(Object.values(result).every(value => typeof value === 'number' || typeof value === 'boolean'));
  assert.doesNotMatch(JSON.stringify(result), /private|example\.com|https:|base64/);
});
test('separates loaded SVG fallbacks from blobs and counts hidden/transparent layout without reading contents', () => {
  const svg = image('data:image/svg+xml,<svg>private name</svg>');
  const hidden = image('blob:https://teams.cloud.microsoft/private-id');
  hidden.styleStub = {display: 'none', visibility: 'hidden', opacity: '1'};
  hidden.parentElement = {styleStub: {display: 'block', visibility: 'visible', opacity: '0'}};
  hidden.getBoundingClientRect = () => ({width: 0, height: 0});
  const result = snapshot({images: [svg, hidden, image('data:image/png;base64,private', {width: 1})]});
  assert.equal(result.loaded_avatar_svg_data, 1); assert.equal(result.loaded_avatar_blob, 1);
  assert.equal(result.loaded_avatar_other_data, 1); assert.equal(result.avatar_tiny_image, 1);
  assert.equal(result.avatar_css_hidden, 1); assert.equal(result.avatar_css_transparent, 1);
  assert.equal(result.avatar_no_box, 1); assert.equal(result.avatar_in_view, 2);
  assert.doesNotMatch(JSON.stringify(result), /private|name|svg>/);
});
test('reads only named photo state and backing flags, without tokens, subscriptions or mutations', () => {
  const keys = [];
  const services = {stateManagementServices: {userWindowStore: {get(key) {
    keys.push(key); return key === 'baseImageUrl' ? 'https://example.com/private?token=secret' : false;
  }, set() {throw Error('must not modify state');}}},
  mediaAuthService: {_isInitialized: true, _isSetupImageCookieInProgress: false,
    _baseImageUrlSubject: {_value: 'https://example.com/private'}, _isImageAuthCookieValidSubject: {_value: false}},
  get authenticationService() {throw Error('must not read authentication data');}};
  const fiber = {memoizedProps: {value: {coreServices: services}}, sibling: null}; fiber.child = fiber;
  const result = snapshot({reactRoot: {'__reactContainer$test': fiber}});
  assert.deepEqual(keys, ['baseImageUrl', 'isImageAuthCookieValid']);
  assert.equal(result.photo_context_found, true); assert.equal(result.photo_base_url_present, true);
  assert.equal(result.photo_cookie_state_known, true); assert.equal(result.photo_cookie_valid, false);
  assert.equal(result.media_auth_service_found, true); assert.equal(result.media_auth_initialized, true);
  assert.equal(result.media_auth_cookie_state_known, true); assert.equal(result.media_auth_cookie_valid, false);
  assert.doesNotMatch(JSON.stringify(result), /secret|private|example\.com|token/);
});
test('does not execute provider getters or treat absent photo state as authoritative false', () => {
  const fiber = {memoizedProps: {value: {get coreServices() {throw Error('do not call');}}}};
  const result = snapshot({reactRoot: {'__reactContainer$test': fiber}});
  assert.equal(result.photo_context_found, false); assert.equal(result.photo_cookie_state_known, false);
  assert.equal(result.media_auth_service_found, false); assert.equal(result.media_auth_cookie_state_known, false);
});
test('records only HTTP status buckets and treats absent or opaque status as unknown', () => {
  const entries = [200, 401, 403, 404, 429, 503, 302, 0, undefined].map(responseStatus => ({
    name: 'https://example.com/users/private-person/photo?token=private-token', initiatorType: 'fetch', responseStatus,
    get headers() {throw Error('must not inspect headers');}
  }));
  entries.push({name: 'https://example.com/avatar.js', initiatorType: 'script', responseStatus: 200});
  const result = snapshot({entries});
  assert.equal(result.photo_requests, 9); assert.equal(result.photo_http_unknown, 2);
  for (const key of ['success', '401', '403', '404', '429', 'server_error', 'other']) assert.equal(result['photo_http_' + key], 1);
  assert.doesNotMatch(JSON.stringify(result), /private|token|example\.com|users/);
});
test('does not inspect auth pages or subframes; handles missing timing without guessing failure', () => {
  assert.equal(snapshot({origin: 'https://login.example.com'}), null);
  assert.equal(snapshot({frame: true}), null);
  const result = snapshot({timing: false});
  assert.equal(result.resource_timing_available, false); assert.equal(result.photo_requests, 0);
});
test('bounds image/resource processing and performs no fetch or image retries', () => {
  const result = snapshot({images: Array.from({length: 1200}, () => image('https://example.com/private-photo')),
    entries: Array.from({length: 1200}, () => ({name: 'https://example.com/photo', initiatorType: 'img', responseStatus: 200}))});
  assert.equal(result.image_scanned, 1000); assert.equal(result.photo_requests, 1000);
});
