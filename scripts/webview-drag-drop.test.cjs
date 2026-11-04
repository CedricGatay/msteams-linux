// Policy-wiring regressions, not a substitute for an actual OS file-drop test.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '..');
const runtime = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8');

test('main runtime explicitly disables native interception before building the webview', () => {
  const creation = runtime.slice(runtime.indexOf('fn create_window'), runtime.indexOf('/// Passive capability probe'));
  const policy = /let builder = if matches!\(kind, WindowKind::Main\)\s*\{\s*builder\.disable_drag_drop_handler\(\)\s*\}\s*else\s*\{\s*builder\s*\};/;
  const match = policy.exec(creation);
  assert.ok(match, 'Teams must receive browser-native HTML5 drops, without changing auth-popup policy');
  assert.ok(match.index < creation.indexOf('builder.inner_size(1280.0, 820.0).center().build()'));
});
test('configurator keeps its native JSON picker and no webpage filesystem permission is added', () => {
  const configurator = fs.readFileSync(path.join(root, 'src-tauri/src/configurator.rs'), 'utf8');
  assert.doesNotMatch(configurator, /disable_drag_drop_handler/);
  const directory = path.join(root, 'src-tauri/capabilities');
  for (const name of fs.readdirSync(directory).filter(name => name.endsWith('.json'))) {
    const capability = JSON.parse(fs.readFileSync(path.join(directory, name), 'utf8'));
    for (const permission of capability.permissions || []) {
      const identifier = typeof permission === 'string' ? permission : permission.identifier;
      assert.doesNotMatch(identifier, /^(?:fs:|shell:)|allow-(?:read|write)-file/, name);
    }
  }
});
