// Count keys only. Never read or return a storage value or a key name.
(function () {
  var result = {
    local_entries: null,
    session_entries: null,
    local_msal_entries: null,
    session_msal_entries: null
  };
  ["local", "session"].forEach(function (kind) {
    try {
      var keys = Object.keys(window[kind + "Storage"]);
      result[kind + "_entries"] = keys.length;
      result[kind + "_msal_entries"] = keys.filter(function (key) {
        return key.toLowerCase().includes("msal");
      }).length;
    } catch (_) {
      // Storage is unavailable on about:blank or when denied by the browser.
    }
  });
  return result;
})();
