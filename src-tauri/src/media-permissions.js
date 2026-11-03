// Request capture permissions in the main Teams page, never on SSO pages.
// Streams are used only to obtain permission and are stopped immediately.
(function () {
  if (window.top !== window || !window.isSecureContext) return;
  var allowedOrigins = window.__msteamsAllowedNotificationOrigins;
  if (Array.isArray(allowedOrigins) ? !allowedOrigins.includes(location.origin)
      : !["teams.cloud.microsoft", "teams.microsoft.com"].includes(location.hostname)) return;
  if (!navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== "function") return;
  if (window.__msteamsRequestMediaPermissions) return;

  var requesting = false;
  async function requestPermissions() {
    if (requesting) return;
    requesting = true;
    var results = window.__msteamsMediaPermissionResults = {};
    try {
      for (var kind of ["audio", "video"]) {
        try {
          // Request separately: a missing camera must not block microphone access.
          var constraints = {};
          constraints[kind] = true;
          var stream = await navigator.mediaDevices.getUserMedia(constraints);
          stream.getTracks().forEach(function (track) { track.stop(); });
          results[kind] = "granted";
        } catch (error) {
          results[kind] = error.name;
        }
      }
    } finally {
      requesting = false;
    }
  }
  // The tray menu can explicitly retry after an interrupted request or a change
  // to macOS Privacy & Security. Never cache an attempted request as a grant.
  window.__msteamsRequestMediaPermissions = requestPermissions;

  if (document.readyState === "complete") {
    requestPermissions();
  } else {
    window.addEventListener("load", requestPermissions, {once: true});
  }
})();
