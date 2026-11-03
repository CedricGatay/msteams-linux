// Teams page notifications -> native OS notifications. No message contents in logs.
(function () {
  if (window.top !== window || !["teams.cloud.microsoft", "teams.microsoft.com"].includes(location.hostname)) return;
  if (window.__msteamsHooked) return;
  window.__msteamsHooked = true;

  var permission = "default";
  var requestInFlight = null;
  var notifications = new Map();
  var tags = new Map();
  var sequence = 0;
  var pageId = window.crypto && window.crypto.randomUUID ? window.crypto.randomUUID() : Date.now().toString(36);
  var deliveryQueue = Promise.resolve();
  var sessionCount = 0;
  var titleCount = 0;
  var last = -1;
  var traceEnabled = window.__msteamsNotificationTraceEnabled === true;
  var trace = {window_calls: 0, service_worker_page_calls: 0, teams_foreground_calls: 0, send_attempts: 0,
    accepted: 0, failed: 0, closed: 0, retained_after_close: 0, clicks: 0, permission_errors: 0, permission_reads: 0, permission_request_calls: 0,
    dom_alert_batches: 0, button_clicks: 0, events: []};
  var serviceWorkerHook = null;
  function focused() { return typeof document.hasFocus === "function" && document.hasFocus(); }
  function mark(stage) {
    if (!traceEnabled) return;
    trace.events.push({stage: stage, t_ms: Date.now(), focused: focused(), visible: !document.hidden, permission: permission});
    if (trace.events.length > 64) trace.events.shift();
  }

  function invoke(command, args) {
    try { return Promise.resolve(window.__TAURI_INTERNALS__.invoke(command, args)); }
    catch (error) { return Promise.reject(error); }
  }
  function warn() { console.warn("Teams native notification bridge operation failed"); }
  function permissionInvoke(request) {
    return invoke("notification_permission", {request: request}).catch(function (error) {
      trace.permission_errors++;
      mark("permission_error");
      throw error;
    });
  }
  function setPermission(value) {
    permission = ["granted", "denied", "default"].includes(value) ? value : "default";
    mark("permission");
    return permission;
  }
  function requestPermission() {
    if (!requestInFlight) {
      requestInFlight = permissionInvoke(true)
        .then(setPermission).finally(function () { requestInFlight = null; });
    }
    return requestInFlight;
  }
  // Wait for navigation to commit before IPC checks the native window origin.
  // This only gates notification delivery, never navigation or application exit.
  var loaded = document.readyState === "complete" ? Promise.resolve() : new Promise(function (resolve) {
    window.addEventListener("load", resolve, {once: true});
  });
  // Ask macOS once on first use; never claim permission is granted before consent.
  var ready = loaded.then(function () {
    return permissionInvoke(false);
  }).then(function (value) {
    setPermission(value);
    return permission === "default" ? requestPermission() : permission;
  });
  ready.catch(warn);

  function report() {
    var count = Math.max(sessionCount, titleCount);
    if (count === last) return;
    last = count;
    invoke("set_unread_count", {count: count}).catch(warn);
  }
  function scanTitle() {
    var match = /^\(\s*(\d+)\s*\)/.exec(document.title || "");
    var count = match ? parseInt(match[1], 10) : 0;
    if (count !== titleCount) { titleCount = count; report(); }
  }
  function forget(notification) {
    if (notifications.get(notification._id) === notification) notifications.delete(notification._id);
    if (notification.tag && tags.get(notification.tag) === notification) tags.delete(notification.tag);
  }

  class TeamsNotification extends EventTarget {
    constructor(title, options, source = "window") {
      super();
      if (source === "service_worker_page") { trace.service_worker_page_calls++; mark("service_worker_page"); }
      else if (source === "teams_foreground") { trace.teams_foreground_calls++; mark("teams_foreground"); }
      else { trace.window_calls++; mark("window_notification"); }
      options = options || {};
      this.title = String(title == null ? "Microsoft Teams" : title);
      this.body = String(options.body == null ? "" : options.body);
      this.tag = String(options.tag || "");
      this.silent = options.silent === true;
      this.data = options.data == null ? null : options.data;
      this.onclick = this.onshow = this.onerror = this.onclose = null;
      this._closed = false;
      this._foreground = source === "teams_foreground";
      this._submitted = false;
      this._nativeRetained = false;
      this._disposed = false;
      var previous = this.tag && tags.get(this.tag);
      this._id = previous ? previous._id : "page-" + pageId + "-" + (++sequence);
      if (previous) {
        var wasClosed = previous._closed;
        previous._closed = true;
        previous._nativeRetained = false;
        forget(previous);
        if (!wasClosed) previous._event("close");
      }
      notifications.set(this._id, this);
      if (this.tag) tags.set(this.tag, this);
      while (notifications.size > 256) forget(notifications.values().next().value);
      this._delivery = deliveryQueue.catch(function () {}).then(() => this._deliver());
      // Constructor callers have no promise to catch. Service-worker page callers
      // receive the original rejection through showNotification's returned promise.
      this._delivery.catch(function () {});
      deliveryQueue = this._delivery;
    }

    static get permission() { trace.permission_reads++; mark("permission_read"); return permission; }
    static get maxActions() { return 0; }
    static requestPermission(callback) {
      trace.permission_request_calls++;
      mark("permission_request");
      return requestPermission().then(function (value) {
        if (typeof callback === "function") callback(value);
        return value;
      });
    }
    _event(type) {
      var event = new Event(type);
      this.dispatchEvent(event);
      var handler = this["on" + type];
      if (typeof handler === "function") handler.call(this, event);
    }
    async _deliver() {
      try {
        await ready.catch(function () {});
        if (this._closed) return;
        if (permission !== "granted") throw new Error("Native notifications are not permitted");
        this._submitted = true;
        trace.send_attempts++;
        mark("send_attempt");
        await invoke("show_notification", {id: this._id, title: this.title, body: this.body, silent: this.silent});
        trace.accepted++;
        mark("delivery_accepted");
        if (this._closed) {
          if (!this._nativeRetained) this._removeNative();
          return;
        }
        sessionCount += 1;
        report();
        this._event("show");
      } catch (error) {
        trace.failed++;
        mark("delivery_failed");
        if (!this._closed) this._event("error");
        if (this._nativeRetained) this._discard();
        warn();
        throw error;
      }
    }
    close() {
      if (this._closed) return;
      this._closed = true;
      trace.closed++;
      mark("closed");
      // Teams expires its in-app UI after ~5s. Do not withdraw an already
      // submitted foreground banner before macOS has had time to present it.
      this._nativeRetained = this._foreground && this._submitted && !this._disposed;
      if (this._nativeRetained) {
        trace.retained_after_close++;
        mark("native_retained");
      } else {
        forget(this);
        this._removeNative();
      }
      this._event("close");
    }
    _removeNative() {
      // A newer tagged replacement owns the same native identifier.
      if (!notifications.has(this._id) || notifications.get(this._id) === this) {
        invoke("close_notification", {id: this._id}).catch(warn);
      }
    }
    _discard() {
      // Adapter rollback/user cancellation must still remove partial delivery.
      if (this._disposed) return;
      this._disposed = true;
      this._nativeRetained = false;
      if (!this._closed) this.close();
      else { forget(this); this._removeNative(); }
    }
  }

  window.Notification = TeamsNotification;
  window.__msteamsForegroundBridge = Object.freeze({
    permission: function () { return permission; },
    create: function (title, options) { return new TeamsNotification(title, options, "teams_foreground"); }
  });
  window.__msteamsNotificationClick = function (id) {
    var notification = notifications.get(id);
    if (notification && (!notification._closed || notification._nativeRetained)) {
      trace.clicks++;
      mark("native_click");
      notification._event("click");
    }
  };
  // This covers registration.showNotification calls made by page code. WebKit
  // worker globals are independent: worker-only push after Quit is not bridged.
  try {
    if (typeof ServiceWorkerRegistration !== "undefined") {
      serviceWorkerHook = function (title, options) {
        try { return new TeamsNotification(title, options, "service_worker_page")._delivery; }
        catch (error) { return Promise.reject(error); }
      };
      ServiceWorkerRegistration.prototype.showNotification = serviceWorkerHook;
    }
  } catch (_) { warn(); }

  // Only metadata is exposed to the Rust trace reader: no titles, bodies, tags,
  // IDs, storage values, DOM text, button labels or worker URLs.
  window.__msteamsNotificationTraceSnapshot = function () {
    if (!traceEnabled) return null;
    var workerControlled = false;
    try { workerControlled = !!(navigator.serviceWorker && navigator.serviceWorker.controller); } catch (_) {}
    return {
      notification_hooked: window.Notification === TeamsNotification,
      service_worker_page_hooked: !!serviceWorkerHook && ServiceWorkerRegistration.prototype.showNotification === serviceWorkerHook,
      worker_controlled: workerControlled,
      iframe_count: typeof document.querySelectorAll === "function" ? document.querySelectorAll("iframe").length : 0,
      permission: permission, focused: focused(), visible: !document.hidden,
      window_calls: trace.window_calls, service_worker_page_calls: trace.service_worker_page_calls,
      teams_foreground_calls: trace.teams_foreground_calls,
      teams_adapter: window.__msteamsForegroundAdapter ? window.__msteamsForegroundAdapter.snapshot() : null,
      send_attempts: trace.send_attempts, accepted: trace.accepted, failed: trace.failed,
      closed: trace.closed, retained_after_close: trace.retained_after_close, clicks: trace.clicks, permission_errors: trace.permission_errors,
      permission_reads: trace.permission_reads, permission_request_calls: trace.permission_request_calls,
      dom_alert_batches: trace.dom_alert_batches, button_clicks: trace.button_clicks,
      title_unread: titleCount, events: trace.events.slice()
    };
  };
  loaded.then(function () {
    if (!traceEnabled) return;
    // ARIA alert/status activity is only a heuristic for in-app UI, not proof
    // of a notification. Never read or log its text or identifying attributes.
    var selector = '[role="alert"], [role="status"]';
    try {
      new MutationObserver(function (records) {
        var activity = records.some(function (record) {
          if (record.target && typeof record.target.closest === "function" && record.target.closest(selector)) return true;
          return Array.from(record.addedNodes || []).some(function (node) {
            return node.nodeType === 1 && (node.matches(selector) || node.querySelector(selector));
          });
        });
        if (activity) { trace.dom_alert_batches++; mark("dom_alert_activity"); }
      }).observe(document.documentElement, {subtree: true, childList: true});
    } catch (_) {}
    document.addEventListener("click", function (event) {
      if (event.target && typeof event.target.closest === "function" && event.target.closest('button, [role="button"]')) {
        trace.button_clicks++;
        mark("button_click");
      }
    }, true);
  }).catch(warn);
  mark("bridge_ready");

  try {
    new MutationObserver(scanTitle).observe(document.head || document.documentElement, {
      subtree: true, childList: true, characterData: true
    });
  } catch (_) {}
  function focus() {
    mark("focus");
    sessionCount = 0;
    report();
    // Refresh after the user changes Allow Notifications in macOS settings.
    permissionInvoke(false).then(setPermission).catch(warn);
  }
  window.addEventListener("focus", focus);
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) focus();
  });
  setInterval(scanTitle, 3000);
  setTimeout(scanTitle, 1500);
})();
