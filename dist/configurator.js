(function () {
  var open = document.getElementById('open'), build = document.getElementById('export');
  var launch = document.getElementById('launch'), name = document.getElementById('name');
  var summary = document.getElementById('summary'), status = document.getElementById('status');
  var result = document.getElementById('result');
  var selected = false, generated = false;
  function invoke(command, args) { return window.__TAURI_INTERNALS__.invoke(command, args || {}); }
  function message(text, error) { status.textContent = String(text || ''); status.className = error ? 'error' : ''; }
  function busy(value) { open.disabled = value; build.disabled = value || !selected; launch.disabled = value || !generated; name.disabled = value; }
  async function choose() {
    selected = false; generated = false; name.value = ''; result.textContent = ''; summary.textContent = 'No configuration selected.';
    busy(true); message('Choose a configuration file.');
    try {
      var config = await invoke('configurator_open_file');
      if (!config) { message('Selection cancelled. Open a file when ready.'); return; }
      selected = true; name.value = config.name;
      summary.textContent = 'Start page: ' + config.teams_url + '\nAdditional sign-in hosts: ' + config.trusted_auth_host_count;
      message('Configuration validated. Choose Create app to select its output location.');
    } catch (error) { message(error, true); }
    finally { busy(false); }
  }
  open.addEventListener('click', choose);
  build.addEventListener('click', async function () {
    busy(true); generated = false; result.textContent = ''; message('Choose a destination outside the source checkout.');
    try {
      var app = await invoke('configurator_export', {name: name.value});
      if (!app) { message('Export cancelled.'); return; }
      generated = true;
      result.textContent = app.path + '\nBundle ID: ' + app.identifier + '\nProfile: ' + app.profile_id;
      message('Application created, signed and verified. It can run independently of this configurator.');
    } catch (error) { message(error, true); }
    finally { busy(false); }
  });
  launch.addEventListener('click', async function () {
    try { await invoke('configurator_open_generated'); message('Application launched.'); }
    catch (error) { message(error, true); }
  });
  invoke('configurator_status').then(function (error) { if (error) message(error, true); })
    .catch(function () { message('Configuration screen could not initialize.', true); });
  // Native selection opens immediately; cancelling leaves a usable screen.
  choose();
})();
