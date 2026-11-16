(function () {
  var open = document.getElementById('open'), build = document.getElementById('export');
  var launch = document.getElementById('launch'), name = document.getElementById('name');
  var summary = document.getElementById('summary'), status = document.getElementById('status');
  var result = document.getElementById('result');
  var application = document.getElementById('application');
  var serviceButtons = document.querySelectorAll('[data-application]');
  var services = {
    teams: {name: 'Microsoft Teams', url: 'https://teams.cloud.microsoft/v2/'},
    outlook: {name: 'Microsoft Outlook', url: 'https://outlook.office.com/mail/'},
    word: {name: 'Microsoft Word', url: 'https://word.cloud.microsoft/'},
    excel: {name: 'Microsoft Excel', url: 'https://excel.cloud.microsoft/'},
    powerpoint: {name: 'Microsoft PowerPoint', url: 'https://powerpoint.cloud.microsoft/'}
  };
  var selected = false, generated = false, original = null;
  function describe() {
    if (!original) return;
    var url = application.value === original.application ? original.teams_url :
      services[application.value].url;
    summary.textContent = 'Start page: ' + url + '\nAdditional sign-in hosts: ' + original.trusted_auth_host_count;
  }
  function invoke(command, args) { return window.__TAURI_INTERNALS__.invoke(command, args || {}); }
  function message(text, error) { status.textContent = String(text || ''); status.className = error ? 'error' : ''; }
  function busy(value) {
    open.disabled = value; build.disabled = value || !selected; launch.disabled = value || !generated;
    name.disabled = value; application.disabled = value || !selected;
    serviceButtons.forEach(function (button) {
      button.disabled = value || !selected;
      button.setAttribute('aria-pressed', String(selected && button.dataset.application === application.value));
    });
  }
  async function choose() {
    selected = false; generated = false; original = null; name.value = ''; result.textContent = ''; summary.textContent = 'No configuration selected.';
    busy(true); message('Choose a configuration file.');
    try {
      var config = await invoke('configurator_open_file');
      if (!config) { message('Selection cancelled. Open a file when ready.'); return; }
      selected = true; original = config; original.application = config.application || 'teams';
      application.value = original.application; name.value = config.name;
      describe();
      message('Configuration validated. Choose Create app to select its output location.');
    } catch (error) { message(error, true); }
    finally { busy(false); }
  }
  open.addEventListener('click', choose);
  function changeApplication() {
    if (!original || !services[application.value]) return;
    if (Object.keys(services).some(function (key) { return name.value === services[key].name; })) {
      name.value = services[application.value].name;
    }
    generated = false; result.textContent = ''; describe(); busy(false);
    message('Choose a separate output for each app. Organization sign-in hosts are preserved.');
  }
  application.addEventListener('change', changeApplication);
  serviceButtons.forEach(function (button) {
    button.addEventListener('click', function () {
      if (button.disabled) return;
      application.value = button.dataset.application;
      changeApplication();
    });
  });
  build.addEventListener('click', async function () {
    busy(true); generated = false; result.textContent = ''; message('Choose a destination outside the source checkout.');
    try {
      var app = await invoke('configurator_export', {name: name.value, application: application.value});
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
