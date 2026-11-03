const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src-tauri/src/storage-counts.js'), 'utf8');

function keysOnlyStore(keys) {
  const store = {};
  for (const key of keys) {
    Object.defineProperty(store, key, {
      enumerable: true,
      get() { throw new Error('Storage values must never be read'); },
    });
  }
  return store;
}

test('counts storage keys without accessing values or returning keys', () => {
  const result = vm.runInNewContext(source, {window: {
    localStorage: keysOnlyStore(['msal.private-account-id', 'unrelated-secret-key']),
    sessionStorage: keysOnlyStore(['msal.token-id']),
  }});
  assert.deepEqual(JSON.parse(JSON.stringify(result)), {
    local_entries: 2, session_entries: 1, local_msal_entries: 1, session_msal_entries: 1,
  });
  assert.ok(!JSON.stringify(result).includes('private-account-id'));
  assert.ok(!JSON.stringify(result).includes('secret-key'));
});

test('reports unavailable storage without logging errors or accessing values', () => {
  const window = {};
  Object.defineProperty(window, 'localStorage', {get() { throw new Error('private-error'); }});
  Object.defineProperty(window, 'sessionStorage', {get() { throw new Error('private-error'); }});
  const result = vm.runInNewContext(source, {window});
  assert.deepEqual(JSON.parse(JSON.stringify(result)), {
    local_entries: null, session_entries: null, local_msal_entries: null, session_msal_entries: null,
  });
});
