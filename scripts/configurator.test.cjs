const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../dist/configurator.js'), 'utf8');
const settle = () => new Promise(resolve => setImmediate(resolve));
function fixture(selection = null, exportResult = null) {
  const elements = {}, calls = [];
  for (const id of ['open', 'export', 'launch', 'name', 'summary', 'status', 'result']) {
    elements[id] = {disabled: false, value: '', textContent: '', handlers: {},
      addEventListener(event, callback) {this.handlers[event] = callback;}};
    Object.defineProperty(elements[id], 'innerHTML', {set() {throw new Error('Never render configuration as HTML');}});
  }
  vm.runInNewContext(source, {document: {getElementById: id => elements[id]},
    window: {__TAURI_INTERNALS__: {invoke: async (command, args) => {
      calls.push({command, args});
      if (command === 'configurator_open_file') return selection;
      if (command === 'configurator_export') return exportResult;
      return null;
    }}}});
  return {elements, calls};
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

test('configurator capability is local-only and not granted to remote runtime windows', () => {
  const capability = JSON.parse(fs.readFileSync(path.join(__dirname, '../src-tauri/capabilities/configurator.json'), 'utf8'));
  assert.deepEqual(capability.windows, ['configurator']);
  assert.equal(capability.local, true);
  assert.equal(capability.remote, undefined);
});
