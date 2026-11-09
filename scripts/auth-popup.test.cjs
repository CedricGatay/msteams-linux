// Wiring checks complement startup-smoke's actual WebKit window.open test.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '..');
const runtime = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8');
const creation = runtime.slice(runtime.indexOf('fn create_window'), runtime.indexOf('/// Passive capability probe'));

test('trusted popup forwards the native request features instead of discarding them', () => {
  const handler = creation.slice(creation.indexOf('.on_new_window('), creation.indexOf('.on_permission_request('));
  assert.match(handler, /on_new_window\(move \|url, features\|/);
  assert.match(handler, /if is_trusted\(&url\)/);
  assert.match(handler, /WindowKind::Popup,[\s\S]*?Some\(features\)/);
  assert.match(handler, /Err\(_\) => NewWindowResponse::Deny/);
  assert.match(handler, /else \{\s*open_external\(&popup_app, &url\);\s*NewWindowResponse::Deny/);
});

test('macOS main selects a profile while popup must inherit WebKit configuration', () => {
  assert.match(creation, /features: Option<NewWindowFeatures>/);
  assert.match(creation, /#\[cfg\(target_os = "macos"\)\]\s*let builder = match kind \{\s*WindowKind::Main => builder\.data_store_identifier\(configuration::current\(\)\.profile\(\)\),\s*WindowKind::Popup => builder\.window_features\(/);
  assert.match(creation, /features\.ok_or_else\(/);
  assert.match(creation, /std::io::ErrorKind::InvalidInput/);
  assert.match(runtime, /create_window\(app\.handle\(\), "main", url, WindowKind::Main, data_dir, None\)/);
});

test('macOS only navigates the main window explicitly, leaving popup navigation to WebKit', () => {
  const nativeNavigation = creation.slice(creation.lastIndexOf('#[cfg(target_os = "macos")]'), creation.indexOf('#[cfg(not(any(target_os = "linux", target_os = "macos")))]'));
  assert.match(nativeNavigation, /if matches!\(kind, WindowKind::Main\) \{\s*app\.state::<SessionRestore>\(\)\.navigate\(&window, url\);\s*\}/);
  assert.doesNotMatch(nativeNavigation, /else|window\.navigate/);
  assert.match(creation, /configure_webkit\(&window, url\)/, 'Linux cookie-policy setup must remain intact');
});
