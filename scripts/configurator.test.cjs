const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../dist/configurator.js'), 'utf8');
const settle = () => new Promise(resolve => setImmediate(resolve));
function fixture(selection = null, exportResult = null) {
  const elements = {}, calls = [];
  for (const id of ['open', 'export', 'launch', 'name', 'application', 'summary', 'status', 'result']) {
    elements[id] = {disabled: false, value: '', textContent: '', handlers: {},
      addEventListener(event, callback) {this.handlers[event] = callback;}};
    Object.defineProperty(elements[id], 'innerHTML', {set() {throw new Error('Never render configuration as HTML');}});
  }
  const buttons = ['teams', 'outlook', 'word', 'excel', 'powerpoint'].map(application => ({
    dataset: {application}, disabled: false, attributes: {}, handlers: {},
    setAttribute(key, value) {this.attributes[key] = value;},
    addEventListener(event, callback) {this.handlers[event] = callback;}
  }));
  vm.runInNewContext(source, {document: {getElementById: id => elements[id], querySelectorAll: () => buttons},
    window: {__TAURI_INTERNALS__: {invoke: async (command, args) => {
      calls.push({command, args});
      if (command === 'configurator_open_file') return selection;
      if (command === 'configurator_export') return exportResult;
      return null;
    }}}});
  return {elements, calls, buttons};
}

test('immediately opens configuration picker and cancellation leaves usable screen', async () => {
  const f = fixture(); await settle();
  assert.equal(f.calls.filter(call => call.command === 'configurator_open_file').length, 1);
  assert.equal(f.elements.open.disabled, false);
  assert.equal(f.elements.export.disabled, true);
  assert.match(f.elements.status.textContent, /cancelled/);
});

test('validated settings are text-only and export is an explicit action', async () => {
  const f = fixture({name: '<b>Example</b>', teams_url: 'https://teams.cloud.microsoft/v2/', trusted_auth_host_count: 2},
    {path: '/private/Example.app', identifier: 'com.example.generated', profile_id: 'synthetic'});
  await settle();
  assert.equal(f.elements.name.value, '<b>Example</b>');
  assert.equal(f.elements.export.disabled, false);
  assert.equal(f.calls.filter(call => call.command === 'configurator_export').length, 0);
  await f.elements.export.handlers.click();
  assert.equal(f.elements.launch.disabled, false);
  assert.match(f.elements.result.textContent, /Example\.app/);
  await f.elements.launch.handlers.click();
  const launch = f.calls.find(call => call.command === 'configurator_open_generated');
  assert.deepEqual(Object.keys(launch.args), []); // No caller-supplied executable/path.
});

test('Teams settings can export Outlook and switching back restores the original landing page', async () => {
  const f = fixture({name: 'Microsoft Teams', teams_url: 'https://teams.microsoft.com/v2/', trusted_auth_host_count: 2},
    {path: '/private/Outlook.app', identifier: 'org.9h41.outlook.generated.synthetic', profile_id: 'synthetic'});
  await settle();
  assert.equal(f.elements.application.value, 'teams');
  f.elements.application.value = 'outlook';
  f.elements.application.handlers.change();
  assert.equal(f.elements.name.value, 'Microsoft Outlook');
  assert.match(f.elements.summary.textContent, /https:\/\/outlook.office.com\/mail\//);
  await f.elements.export.handlers.click();
  const exported = f.calls.find(call => call.command === 'configurator_export');
  assert.equal(exported.args.application, 'outlook');
  assert.equal(exported.args.name, 'Microsoft Outlook');
  f.elements.application.value = 'teams';
  f.elements.application.handlers.change();
  assert.equal(f.elements.name.value, 'Microsoft Teams');
  assert.equal(f.elements.launch.disabled, true);
  assert.equal(f.elements.result.textContent, '');
  assert.match(f.elements.summary.textContent, /https:\/\/teams.microsoft.com\/v2\//);
});

test('Outlook configs select Outlook automatically and custom names survive switching', async () => {
  const f = fixture({application: 'outlook', name: 'Work Mail', teams_url: 'https://outlook.live.com/mail/', trusted_auth_host_count: 1});
  await settle();
  assert.equal(f.elements.application.value, 'outlook');
  assert.equal(f.elements.name.value, 'Work Mail');
  f.elements.application.value = 'teams';
  f.elements.application.handlers.change();
  assert.equal(f.elements.name.value, 'Work Mail');
  f.elements.application.value = 'outlook';
  f.elements.application.handlers.change();
  assert.match(f.elements.summary.textContent, /https:\/\/outlook.live.com\/mail\//);
});

test('service icon buttons export all supported services and match their preset landing pages', async () => {
  const f = fixture({name: 'Microsoft Teams', teams_url: 'https://teams.cloud.microsoft/v2/', trusted_auth_host_count: 2});
  await settle();
  const html = fs.readFileSync(path.join(__dirname, '../dist/configurator.html'), 'utf8');
  for (const button of f.buttons) {
    const service = button.dataset.application;
    assert.equal(button.disabled, false);
    assert.ok(html.includes('data-application="' + service + '"'));
    assert.ok(html.includes('src="services/' + service + '.png"'));
    assert.ok(fs.existsSync(path.join(__dirname, '../dist/services/' + service + '.png')));
    button.handlers.click();
    assert.equal(f.elements.application.value, service);
    assert.equal(button.attributes['aria-pressed'], 'true');
    assert.equal(f.buttons.filter(b => b.attributes['aria-pressed'] === 'true').length, 1);
    const preset = JSON.parse(fs.readFileSync(path.join(__dirname, '../config/' + (service === 'teams' ? 'default' : service) + '.json'), 'utf8'));
    assert.equal(f.elements.name.value, preset.name);
    assert.ok(f.elements.summary.textContent.includes(preset.teams_url));
    await f.elements.export.handlers.click();
    assert.equal(f.calls.filter(c => c.command === 'configurator_export').at(-1).args.application, service);
  }
});

test('icon buttons are disabled with no configuration selected', async () => {
  const f = fixture(); await settle();
  assert.ok(f.buttons.every(button => button.disabled));
  assert.ok(f.buttons.every(button => button.attributes['aria-pressed'] === 'false'));
});

test('Teams-specific page adapters are gated but shared notification and zoom scripts remain available', () => {
  const runtime = fs.readFileSync(path.join(__dirname, '../src-tauri/src/lib.rs'), 'utf8');
  for (const script of ['teams-foreground-adapter.js', 'media-permissions.js', 'screen-presence.js']) {
    const injection = runtime.indexOf('include_str!("' + script + '")');
    assert.ok(injection > 0);
    const before = runtime.slice(runtime.lastIndexOf('let builder =', injection), injection);
    assert.match(before, /application == configuration::Application::Teams/);
  }
  assert.match(runtime, /initialization_script\(INJECT_JS\)/);
  assert.match(runtime, /initialization_script\(include_str!\("zoom-hotkeys.js"\)\)/);
});

test('Teams-only native inspection is guarded and Office does not start a screen observer', () => {
  const runtime = fs.readFileSync(path.join(__dirname, '../src-tauri/src/lib.rs'), 'utf8');
  for (const command of ['screen_presence_status', 'media_permission_status', 'foreground_notifications_enabled']) {
    const start = runtime.indexOf('fn ' + command + '(');
    const body = runtime.slice(start, runtime.indexOf('\n}', start));
    assert.match(body, /require_teams_window\(&window\)\?/);
  }
  assert.match(runtime, /"permissions": configuration::current\(\)\.application\.remote_permissions\(\)/);
  const observer = runtime.indexOf('app.manage(screen_presence::ScreenPresence::new');
  assert.match(runtime.slice(runtime.lastIndexOf('#[cfg(target_os = "macos")]', observer), observer),
    /if configuration::current\(\)\.application == configuration::Application::Teams/);
});

test('configurator capability is local-only and not granted to remote runtime windows', () => {
  const capability = JSON.parse(fs.readFileSync(path.join(__dirname, '../src-tauri/capabilities/configurator.json'), 'utf8'));
  assert.deepEqual(capability.windows, ['configurator']);
  assert.equal(capability.local, true);
  assert.equal(capability.remote, undefined);
});
