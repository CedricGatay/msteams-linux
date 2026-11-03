// Logical-key fallback for Cmd+Plus (including layouts where Shift is needed).
// Native menu accelerators handle Cmd+=, Cmd+- and Cmd+0 before page events.
(function () {
  var allowed = window.__msteamsAllowedNotificationOrigins;
  if (window.top !== window || !Array.isArray(allowed) || !allowed.includes(location.origin)) return;
  if (window.__msteamsZoomKeys) return;
  window.__msteamsZoomKeys = true;
  document.addEventListener('keydown', function (event) {
    if (!event.metaKey || event.ctrlKey || event.altKey || event.isComposing || event.defaultPrevented) return;
    var action = event.key === '+' || event.key === '=' ? 'in' : event.key === '-' ? 'out' : event.key === '0' ? 'reset' : null;
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    try {
      Promise.resolve(window.__TAURI_INTERNALS__.invoke('zoom_teams', {action: action}))
        .catch(function () { console.warn('Teams page zoom operation failed'); });
    } catch (_) { console.warn('Teams page zoom operation failed'); }
  }, true);
})();
