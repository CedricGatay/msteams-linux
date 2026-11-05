// Main Teams page only. Wait for the rendered shell, check permission without
// capture, then offer a non-modal prompt. Only its button can request capture.
(function () {
  if (window.top !== window || !window.isSecureContext) return;
  var allowedOrigins = window.__msteamsAllowedNotificationOrigins;
  if (Array.isArray(allowedOrigins) ? !allowedOrigins.includes(location.origin)
      : !['teams.cloud.microsoft', 'teams.microsoft.com'].includes(location.hostname)) return;
  if (!navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== 'function') return;
  if (window.__msteamsRequestMediaPermissions) return;

  var hostId = '__msteams-media-authorization';
  var ui = null, dismissed = false, requesting = false, checking = null;
  var observer = null, readyTimer = null, deadline = null, stopped = false;

  function bounded(task) {
    var timer;
    return Promise.race([
      Promise.resolve().then(task),
      new Promise(function (_, reject) { timer = setTimeout(function () { reject(new Error('Permission status unavailable')); }, 1000); })
    ]).finally(function () { clearTimeout(timer); });
  }
  async function browserState(name) {
    try {
      if (!navigator.permissions || typeof navigator.permissions.query !== 'function') return 'unknown';
      var status = await bounded(function () { return navigator.permissions.query({name: name}); });
      return status && ['granted', 'denied', 'prompt'].includes(status.state) ? status.state : 'unknown';
    } catch (_) { return 'unknown'; }
  }
  async function nativeState() {
    try {
      if (!window.__msteamsHasNativeMediaStatus || !window.__TAURI_INTERNALS__) return {};
      return await bounded(function () { return window.__TAURI_INTERNALS__.invoke('media_permission_status'); }) || {};
    } catch (_) { return {}; }
  }
  function classify(browser, os) {
    if (browser === 'denied') return 'denied';
    if (os === 'denied' || os === 'restricted') return os;
    if (os === 'authorized') return browser === 'granted' ? 'granted' : 'os-authorized';
    if (os === 'not-determined') return 'prompt';
    if (browser === 'granted') return 'granted';
    if (!window.__msteamsHasNativeMediaStatus && browser === 'prompt') return 'prompt';
    return 'unknown';
  }
  function granted(value) { return value === 'granted' || value === 'os-authorized'; }
  function blocked(value) { return ['denied', 'restricted', 'NotAllowedError', 'SecurityError'].includes(value); }
  async function checkPermissions() {
    if (checking) return checking;
    checking = Promise.all([browserState('microphone'), browserState('camera'), nativeState()]).then(function (states) {
      var results = {audio: classify(states[0], states[2].audio), video: classify(states[1], states[2].video)};
      window.__msteamsMediaPermissionResults = results;
      return results;
    }).finally(function () { checking = null; });
    return checking;
  }

  function removePrompt() {
    if (ui) ui.host.remove();
    ui = null;
  }
  function dismiss() { dismissed = true; removePrompt(); }
  function element(tag, text, styles) {
    var node = document.createElement(tag);
    if (text) node.textContent = text;
    if (styles) Object.assign(node.style, styles);
    return node;
  }
  function ensurePrompt() {
    if (ui && ui.host.parentNode) return ui;
    var host = element('div', null, {position: 'fixed', top: '12px', left: '50%', transform: 'translateX(-50%)',
      width: 'min(560px, calc(100vw - 32px))', zIndex: '2147483000', fontFamily: 'system-ui, sans-serif', fontSize: '14px'});
    host.id = hostId;
    // Isolate from Teams CSS. No HTML strings, remote assets, private DOM text,
    // or stylesheets/CSP changes; all copy is fixed text and styles use CSSOM.
    var shadow = host.attachShadow({mode: 'closed'});
    var card = element('section', null, {background: '#f5f5ff', color: '#20203a', border: '1px solid #9393cc',
      borderRadius: '12px', padding: '16px', boxShadow: '0 4px 20px #0003', lineHeight: '1.45'});
    card.setAttribute('role', 'region'); card.setAttribute('aria-label', 'Teams Shell media permissions');
    card.appendChild(element('strong', 'Teams Shell · Microphone and camera'));
    var status = element('p', '', {margin: '8px 0 12px'});
    status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
    card.appendChild(status);
    var buttons = element('div', null, {display: 'flex', flexWrap: 'wrap', gap: '8px'});
    var enable = element('button', 'Enable access', {font: 'inherit', padding: '8px 12px', borderRadius: '6px',
      border: '1px solid #464775', background: '#464775', color: '#fff', cursor: 'pointer'});
    var later = element('button', 'Not now', {font: 'inherit', padding: '8px 12px', borderRadius: '6px',
      border: '1px solid #888', background: '#fff', color: '#20203a', cursor: 'pointer'});
    enable.type = later.type = 'button';
    enable.addEventListener('click', authorize);
    later.addEventListener('click', function (event) { event.stopPropagation(); dismiss(); });
    buttons.appendChild(enable); buttons.appendChild(later); card.appendChild(buttons); shadow.appendChild(card);
    document.body.appendChild(host);
    ui = {host: host, status: status, enable: enable, canAuthorize: false};
    return ui;
  }
  function render(results) {
    if (dismissed || !document.body) return;
    var prompt = ensurePrompt(), allGranted = granted(results.audio) && granted(results.video);
    prompt.enable.disabled = requesting;
    if (requesting) {
      prompt.status.textContent = prompt.canAuthorize
        ? 'Waiting for browser or macOS authorization. Devices are stopped immediately after a permission request.'
        : 'Checking current permission status without opening devices.';
      prompt.enable.textContent = prompt.canAuthorize ? 'Authorizing…' : 'Checking…';
    } else if (allGranted) {
      prompt.status.textContent = 'Access is already authorized. Devices open only when Teams uses them.';
      prompt.canAuthorize = false;
      prompt.enable.textContent = 'Check again';
    } else if (blocked(results.audio) || blocked(results.video)) {
      prompt.status.textContent = window.__msteamsHasNativeMediaStatus
        ? 'Access is denied or restricted. Review this app in System Settings → Privacy & Security → Microphone / Camera, then check again.'
        : 'Access is denied or restricted. Review browser or system permissions, then check again.';
      prompt.canAuthorize = [results.audio, results.video].some(function (value) { return !granted(value) && !blocked(value); });
      prompt.enable.textContent = prompt.canAuthorize ? 'Enable access' : 'Check again';
    } else {
      prompt.canAuthorize = true;
      prompt.status.textContent = 'Enable access when you are ready for calls. Nothing opens until you choose Enable access; already-authorized devices are skipped.';
      prompt.enable.textContent = 'Enable access';
    }
  }
  async function showPrompt(manual) {
    if (manual) dismissed = false;
    var results = await checkPermissions();
    if (dismissed) return;
    if (!manual && granted(results.audio) && granted(results.video)) { removePrompt(); return; }
    render(results);
  }
  async function authorize(event) {
    if (!event || event.isTrusted !== true) return;
    event.stopPropagation();
    if (requesting) return;
    var canAuthorize = !!(ui && ui.canAuthorize);
    requesting = true;
    try {
      var results = await checkPermissions();
      if (dismissed) return;
      render(results);
      for (var kind of ['audio', 'video']) {
        if (!canAuthorize || dismissed) break;
        // Denial needs settings, not repeated capture requests. Unknown state is
        // allowed to ask ONLY because this explicit in-page button was clicked.
        if (granted(results[kind]) || blocked(results[kind])) continue;
        try {
          var constraints = {}; constraints[kind] = true;
          var stream = await navigator.mediaDevices.getUserMedia(constraints);
          stream.getTracks().forEach(function (track) { try { track.stop(); } catch (_) {} });
          results[kind] = 'granted';
        } catch (error) {
          var name = error && error.name;
          results[kind] = ['NotAllowedError', 'NotFoundError', 'NotReadableError', 'AbortError', 'OverconstrainedError', 'SecurityError', 'TypeError'].includes(name) ? name : 'error';
        }
      }
      window.__msteamsMediaPermissionResults = results;
      if (granted(results.audio) && granted(results.video)) { removePrompt(); return; }
    } finally { requesting = false; }
    if (!dismissed) render(window.__msteamsMediaPermissionResults || {});
  }

  function visibleContent(node) {
    return !!node && node.getAttribute('aria-busy') !== 'true' && node.getAttribute('aria-hidden') !== 'true'
      && node.childElementCount > 0 && node.getClientRects().length > 0;
  }
  function shellReady() {
    if (document.readyState !== 'complete' || document.visibilityState === 'hidden') return false;
    var root = document.getElementById('app');
    if (!root || root.getAttribute('aria-busy') === 'true') return false;
    var main = document.querySelector('[data-tid="app-layout-area--main"], #app [role="main"]');
    var navigation = document.querySelector('[data-tid="app-bar"], [data-tid="app-layout-area--nav"], #app [role="navigation"]');
    return visibleContent(main) && visibleContent(navigation);
  }
  function stopWatching() {
    stopped = true;
    if (observer) observer.disconnect();
    clearTimeout(readyTimer); clearTimeout(deadline);
    document.removeEventListener('visibilitychange', considerReady);
  }
  function considerReady() {
    if (stopped || readyTimer !== null || !shellReady()) return;
    // Give the rendered shell a short settling period, rather than prompting
    // over the login/loading screen. Never wait for arbitrary network idleness.
    readyTimer = setTimeout(function () {
      readyTimer = null;
      if (!shellReady()) return;
      stopWatching();
      showPrompt(false);
    }, 750);
  }
  function watchShell() {
    if (!document.body) return;
    if (typeof MutationObserver === 'function') {
      observer = new MutationObserver(considerReady);
      observer.observe(document.getElementById('app') || document.body, {subtree: true, childList: true, attributes: true, attributeFilter: ['aria-busy', 'aria-hidden']});
    }
    document.addEventListener('visibilitychange', considerReady);
    // If a future Teams layout is not recognized, do not guess or activate
    // hardware. Help remains an explicit fallback for showing the same bar.
    deadline = setTimeout(stopWatching, 120000);
    considerReady();
  }
  // Help reveals the consent bar, never starts capture by itself. It also works
  // as a fallback if a future Teams layout changes the automatic ready markers.
  window.__msteamsRequestMediaPermissions = function () { return showPrompt(true); };
  if (document.readyState === 'complete') watchShell();
  else window.addEventListener('load', watchShell, {once: true});
})();
