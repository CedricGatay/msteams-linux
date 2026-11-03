// Experimental Teams service adapter. Discovery examines only known service
// references in React providers, never message text/stores or arbitrary props.
(function (factory) {
  var createAdapter = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = {createAdapter: createAdapter};
    return;
  }
  var allowedOrigins = window.__msteamsAllowedNotificationOrigins;
  var allowedPage = Array.isArray(allowedOrigins) ? allowedOrigins.includes(location.origin)
    : ["teams.cloud.microsoft", "teams.microsoft.com"].includes(location.hostname);
  if (window.top !== window || !allowedPage) return;
  var adapter = createAdapter({window: window, document: document,
    enabled: false, trace: window.__msteamsNotificationTraceEnabled === true});
  var preferenceRevision = 0;
  window.__msteamsForegroundAdapter = Object.assign({}, adapter, {
    setEnabled: function (value) { preferenceRevision++; adapter.setEnabled(value); }
  });
  function ready() {
    var revision = preferenceRevision;
    adapter.start(); // Trace may observe while the opt-in preference is loading.
    try {
      Promise.resolve(window.__TAURI_INTERNALS__.invoke("foreground_notifications_enabled", {})).then(function (value) {
        if (preferenceRevision === revision) adapter.setEnabled(value === true);
      }).catch(function () {
        if (preferenceRevision === revision) adapter.setEnabled(false);
      });
    } catch (_) { if (preferenceRevision === revision) adapter.setEnabled(false); }
  }
  if (document.readyState === "complete") ready();
  else window.addEventListener("load", ready, {once: true});
  window.addEventListener("focus", function () { adapter.start(); });
})(function () {
  function object(value) { return value !== null && (typeof value === "object" || typeof value === "function"); }
  function own(value, key) {
    if (!object(value)) return undefined;
    try { var descriptor = Object.getOwnPropertyDescriptor(value, key); return descriptor && descriptor.value; }
    catch (_) { return undefined; }
  }
  function method(value, key) {
    for (var depth = 0; object(value) && depth < 5; depth++) {
      var descriptor;
      try { descriptor = Object.getOwnPropertyDescriptor(value, key); } catch (_) { return null; }
      if (descriptor) return typeof descriptor.value === "function" ? descriptor.value : null;
      try { value = Object.getPrototypeOf(value); } catch (_) { return null; }
    }
    return null;
  }

  return function createAdapter(options) {
    var root = options.window;
    var document = options.document;
    var enabled = options.enabled === true;
    var trace = options.trace === true;
    var bindings = [];
    var pending = new WeakMap();
    var timer = null;
    var attempts = 0;
    var setTimer = options.setInterval || setInterval;
    var clearTimer = options.clearInterval || clearInterval;
    var stats = {react_root_found: false, handler_found: false, in_app_service_found: false,
      discovery_attempts: 0, discovery_exhausted: false, browser_attempts: 0, queued: 0,
      skipped_focus: 0, skipped_permissions: 0, other_results: 0, original_errors: 0,
      mirrored: 0, mirror_errors: 0, unsupported: 0, activation_errors: 0,
      in_app_calls: 0, in_app_tests: 0};

    function compatibleHandler(value) {
      return method(value, "tryShowBrowserNotification") && method(value, "isOutOfTabLocalSettingEnabled")
        && method(value, "activateBrowserToast");
    }
    function bind(target, name, wrap) {
      var existing = bindings.find(function (binding) { return binding.target === target && binding.name === name; });
      if (existing && method(target, name) === existing.wrapper) return true;
      if (existing) bindings.splice(bindings.indexOf(existing), 1);
      if (bindings.length >= 32) { stats.unsupported++; return false; }
      var original = method(target, name);
      if (!original) return false;
      var descriptor;
      try {
        descriptor = Object.getOwnPropertyDescriptor(target, name);
        var wrapper = wrap(original);
        var replacement = descriptor ? Object.assign({}, descriptor, {value: wrapper})
          : {value: wrapper, writable: true, configurable: true, enumerable: false};
        Object.defineProperty(target, name, replacement);
        bindings.push({target: target, name: name, wrapper: wrapper, descriptor: descriptor});
        return true;
      } catch (_) { stats.unsupported++; return false; }
    }
    function unbind() {
      bindings.forEach(function (binding) {
        try {
          if (method(binding.target, binding.name) !== binding.wrapper) return;
          if (binding.descriptor) Object.defineProperty(binding.target, binding.name, binding.descriptor);
          else delete binding.target[binding.name];
        } catch (_) { stats.unsupported++; }
      });
      bindings = [];
    }

    function discard(notification) {
      var remove = method(notification, "_discard") || method(notification, "close");
      if (remove) Reflect.apply(remove, notification, []);
    }

    async function mirror(service, toast, record, originalResult) {
      var notification = null, registered = false, previous;
      var register = method(toast, "registerGetNotificationCallback");
      try {
        var payloadMethod = method(toast, "getNotificationOptionsPayload");
        var webOptionsMethod = method(toast, "getWebToastOptions");
        var getNotification = method(toast, "getNotification");
        var activate = method(service, "activateBrowserToast");
        var bridge = root.__msteamsForegroundBridge;
        if (!enabled || !bridge || bridge.permission() !== "granted" || !payloadMethod
            || !webOptionsMethod || !register || !getNotification || !activate) {
          stats.unsupported++; return originalResult;
        }
        var payload = Reflect.apply(payloadMethod, toast, []);
        var nativeOptions = payload && payload.notificationOptions;
        if (!payload || typeof payload.notificationTitle !== "string" || !nativeOptions
            || (nativeOptions.body != null && typeof nativeOptions.body !== "string")) {
          stats.unsupported++; return originalResult;
        }
        var webOptions = Reflect.apply(webOptionsMethod, toast, []);
        if (!object(webOptions) || !enabled) return originalResult;
        previous = Reflect.apply(getNotification, toast, []);
        notification = bridge.create(payload.notificationTitle, {
          body: nativeOptions.body || "", silent: nativeOptions.silent === true,
          tag: typeof nativeOptions.tag === "string" ? nativeOptions.tag : ""
        });
        if (!notification || !notification._delivery || typeof notification.addEventListener !== "function"
            || typeof notification.close !== "function") throw new Error("Unsupported bridge");
        record.notification = notification;
        registered = true;
        Reflect.apply(register, toast, [function () { return notification; }]);
        notification.addEventListener("click", function () {
          try { Promise.resolve(Reflect.apply(activate, service, [webOptions])).catch(function () { stats.activation_errors++; }); }
          catch (_) { stats.activation_errors++; }
        });
        await notification._delivery;
        if (!enabled || (notification._closed && !notification._nativeRetained)) {
          discard(notification);
          Reflect.apply(register, toast, [function () { return previous; }]);
          return originalResult;
        }
        stats.mirrored++;
        record.shown = true;
        return "queued";
      } catch (_) {
        stats.mirror_errors++;
        try { if (notification) discard(notification); } catch (_) {}
        try { if (registered) Reflect.apply(register, toast, [function () { return previous; }]); } catch (_) {}
        return originalResult;
      }
    }

    function observeHandler(handler) {
      stats.handler_found = true;
      bind(handler, "tryShowBrowserNotification", function (original) {
        return function () {
          var service = this, args = arguments, result;
          stats.browser_attempts++;
          try { result = Reflect.apply(original, service, args); }
          catch (error) { stats.original_errors++; throw error; }
          var then = method(result, "then");
          if (!then) { stats.other_results++; return result; }
          return Reflect.apply(then, result, [function (reason) {
            if (reason === "queued") stats.queued++;
            else if (reason === "skippedNoPermissions") stats.skipped_permissions++;
            else if (reason === "skippedAppInFocus") stats.skipped_focus++;
            else stats.other_results++;
            if (reason !== "skippedAppInFocus" || !enabled || !object(args[0])) return reason;
            var toast = args[0], record = pending.get(toast);
            if (record) return record.shown && record.notification && record.notification._closed && !record.notification._nativeRetained ? reason : record.promise;
            record = {shown: false, notification: null, promise: null};
            // Publish the in-flight record before invoking any application method.
            record.promise = Promise.resolve().then(function () { return mirror(service, toast, record, reason); }).then(function (outcome) {
              if (outcome !== "queued") pending.delete(toast);
              return outcome;
            });
            pending.set(toast, record);
            return record.promise;
          }, function (error) { stats.original_errors++; throw error; }]);
        };
      });
    }
    function observeInApp(toasts) {
      stats.in_app_service_found = true;
      if (!trace) return;
      bind(toasts, "showMessage", function (original) {
        return function () {
          stats.in_app_calls++;
          if (arguments[0] === "testNotification") stats.in_app_tests++;
          return Reflect.apply(original, this, arguments);
        };
      });
    }

    function scan() {
      if (!enabled && !trace) return false;
      stats.discovery_attempts++;
      var container = document.getElementById("app");
      if (!container) return false;
      var stack = [], seenFibers = new WeakSet(), seenContexts = new WeakSet();
      try {
        Object.getOwnPropertyNames(container).forEach(function (key) {
          if (key.startsWith("__reactContainer$") || key.startsWith("__reactFiber$")) stack.push(own(container, key));
        });
      } catch (_) { return false; }
      var legacy = own(own(container, "_reactRootContainer"), "_internalRoot");
      if (legacy) stack.push(own(legacy, "current"));
      stats.react_root_found = stack.some(object);
      var fibers = 0, contexts = 0;
      function inspect(value, depth) {
        if (!object(value) || depth > 3 || contexts >= 3000 || seenContexts.has(value)) return;
        seenContexts.add(value); contexts++;
        if (compatibleHandler(value)) observeHandler(value);
        var handler = own(value, "notificationsHandler");
        if (compatibleHandler(handler)) observeHandler(handler);
        var toasts = own(value, "toasts");
        if (method(toasts, "showMessage")) observeInApp(toasts);
        ["coreServices", "context", "defaultContext", "requestContext", "value", "entityCommanding", "commanding"].forEach(function (key) {
          inspect(own(value, key), depth + 1);
        });
      }
      while (stack.length && fibers < 3000) {
        var fiber = stack.pop();
        if (!object(fiber) || seenFibers.has(fiber)) continue;
        seenFibers.add(fiber); fibers++;
        inspect(own(fiber, "memoizedProps"), 0);
        inspect(own(fiber, "pendingProps"), 0);
        stack.push(own(fiber, "child"), own(fiber, "sibling"), own(fiber, "current"), own(own(fiber, "stateNode"), "current"));
      }
      return stats.handler_found;
    }
    function stop() { if (timer !== null) { clearTimer(timer); timer = null; } }
    function start() {
      if ((!enabled && !trace) || timer !== null) return;
      attempts = 0; stats.discovery_exhausted = false;
      scan();
      timer = setTimer(function () {
        scan();
        if (++attempts >= 30) {
          stats.discovery_exhausted = !bindings.some(function (binding) {
            return binding.name === "tryShowBrowserNotification" && method(binding.target, binding.name) === binding.wrapper;
          });
          stop();
        }
      }, 2000);
    }
    return {
      start: start, scan: scan,
      setEnabled: function (value) {
        enabled = value === true;
        if (enabled || trace) start(); else { stop(); unbind(); }
      },
      snapshot: function () {
        return Object.assign({}, stats, {enabled: enabled,
          hooked: bindings.some(function (binding) { return binding.name === "tryShowBrowserNotification" && method(binding.target, binding.name) === binding.wrapper; })});
      },
      stop: function () { stop(); unbind(); }
    };
  };
});
