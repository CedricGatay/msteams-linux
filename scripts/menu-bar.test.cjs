// Wiring checks complement the native menu/reload smoke and preference tests.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '..');
const runtime = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8');
const menus = fs.readFileSync(path.join(root, 'src-tauri/src/menu_bar.rs'), 'utf8');

test('commands move to Help and use application-wide dispatch independent of the tray', () => {
  assert.match(menus, /while let Some\(item\) = actions\.remove_at\(0\)\?\s*\{\s*help\.append\(&item\)\?/);
  assert.match(menus, /root\.get\(HELP_SUBMENU_ID\)/);
  assert.match(runtime, /app\.on_menu_event\(move \|app, event\|/);
  const tray = runtime.slice(runtime.indexOf('let tray = TrayIconBuilder::with_id("main")'), runtime.indexOf('let tray = tray.build(app)?'));
  assert.doesNotMatch(tray, /\.on_menu_event/);
  assert.match(tray, /#\[cfg\(not\(target_os = "macos"\)\)\]\s*let tray = tray\.menu\(&menu\)/);
});
test('View exposes reload shortcut and a persistent native tray checkbox without page IPC', () => {
  assert.match(menus, /RELOAD_ID,\s*"Reload Teams",\s*true,\s*Some\("CmdOrCtrl\+R"\)/);
  assert.match(menus, /CheckMenuItem::with_id/);
  assert.match(menus, /"Show Tray Icon"/);
  assert.match(menus, /tray-visible\.json/);
  assert.match(runtime, /\.set_visible\(visible\)/);
  assert.doesNotMatch(menus, /#\[tauri::command\]/);
});
test('reload preserves active Keychain restoration, reloads Teams and can recover a blank document', () => {
  const reload = runtime.slice(runtime.indexOf('fn reload_main('), runtime.indexOf('fn show_main'));
  assert.match(reload, /SessionRestore::is_restore_page/);
  assert.ok(reload.indexOf('SessionRestore::is_restore_page') < reload.indexOf('window.reload()'));
  assert.match(reload, /window\.reload\(\)/);
  assert.match(reload, /window\s*\.navigate/);
  assert.match(reload, /configuration::current\(\)\.teams_url/);
  assert.doesNotMatch(reload, /\.restart\(|\.exit\(|delete_cookie|clear_data|clear_cache/);
});
test('reload is a production macOS action, not a debug/devtools or private-probe feature', () => {
  assert.match(runtime, /#\[cfg\(target_os = "macos"\)\]\s*fn reload_main/);
  assert.match(runtime, /menu_bar::install\(app\.handle\(\), &menu, data_dir\.clone\(\)\)/);
  assert.doesNotMatch(menus, /debug_assertions|devtools|avatar-probe/);
});
test('Dock reopen restores a hidden window even when the tray is not displayed', () => {
  assert.match(runtime, /RunEvent::Reopen\s*\{\s*has_visible_windows: false/);
  const start = runtime.indexOf('RunEvent::Reopen');
  const reopen = runtime.slice(start, runtime.indexOf('#[cfg(test)]', start));
  assert.match(reopen, /show_main\(_app\)/);
  assert.doesNotMatch(reopen, /tray_by_id/);
});
