// Experimental availability policy: screen saver/lock/session state, not mouse
// or keyboard inactivity. No synthetic input, preferred-presence API or UA spoof.
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module.exports) { module.exports = api; return; }
  if (root.top !== root || !root.isSecureContext || root.__msteamsScreenPresence) return;
  var allowed = root.__msteamsAllowedNotificationOrigins;
  if (!Array.isArray(allowed) || !allowed.includes(root.location.origin)) return;
  var adapter = api.createAdapter({findHost: function () {return api.discover(root.document);},
    setTimer: root.setInterval.bind(root), clearTimer: root.clearInterval.bind(root)});
  var receivedNativePush = false;
  root.__msteamsScreenPresence = {receive: function (value) {receivedNativePush = true; adapter.receive(value);}, getStats: adapter.getStats};
  function readNativeStatus() {
    try {
      Promise.resolve(root.__TAURI_INTERNALS__.invoke('screen_presence_status')).then(function (value) {
        // A slow initial response must not overwrite a newer enable/disable push.
        if (!receivedNativePush) adapter.receive(value);
      }).catch(function () {});
    } catch (_) {}
  }
  // Wait for the committed Teams document so the native origin check cannot
  // race the preceding sign-in/bootstrap URL during document-start injection.
  if (root.document.readyState === 'complete') readNativeStatus();
  else root.addEventListener('load', readNativeStatus, {once:true});
})(typeof window === 'object' ? window : globalThis, function () {
  function object(value) {return value !== null && typeof value === 'object';}
  function own(value, key) {
    if (!object(value)) return undefined;
    try {var descriptor = Object.getOwnPropertyDescriptor(value, key); return descriptor && descriptor.value;} catch (_) {}
  }
  function method(value, key) {
    for (var level = 0; object(value) && level < 4; level++) {
      try {
        var descriptor = Object.getOwnPropertyDescriptor(value, key);
        if (descriptor) return typeof descriptor.value === 'function' ? descriptor.value : null;
        value = Object.getPrototypeOf(value);
      } catch (_) {return null;}
    }
  }
  var stateKeys = ['windowState', 'suspendedState', 'machineLockState', 'machineIdleState'];
  function validState(value) {
    return object(value) && Object.keys(value).length === 4 && Object.keys(value).every(function (key) {return stateKeys.includes(key);})
      && ['Minimized','Background','InFocus'].includes(own(value,'windowState'))
      && ['Initialized','Unknown','Suspending','Resumed'].includes(own(value,'suspendedState'))
      && ['Initialized','Unknown','Locked','Active'].includes(own(value,'machineLockState'))
      && ['Unknown','Idle','Active'].includes(own(value,'machineIdleState'));
  }
  function compatible(host) {
    var subject = own(host, '_hostStateSubject'), get = method(subject, 'getValue'), next = method(subject, 'next');
    if (!method(host, 'getState') || !method(host, 'observe') || !get || !next || own(subject,'closed') === true || own(subject,'isStopped') === true) return false;
    try {return validState(Reflect.apply(get, subject, []));} catch (_) {return false;}
  }
  function discover(document) {
    var container = document.getElementById('app'); if (!container) return null;
    var stack = [], fibers = new WeakSet(), contexts = new WeakSet(), inspected = 0, found = null;
    try {Object.getOwnPropertyNames(container).forEach(function (key) {
      if (key.startsWith('__reactContainer$') || key.startsWith('__reactFiber$')) stack.push(own(container, key));
    });} catch (_) {return null;}
    stack.push(own(own(own(container, '_reactRootContainer'), '_internalRoot'), 'current'));
    function inspect(value, depth) {
      if (found || !object(value) || contexts.has(value) || depth > 4 || inspected++ >= 3000) return;
      contexts.add(value);
      var host = own(value, 'hostState');
      if (compatible(host)) {found = host; return;}
      ['coreServices','context','defaultContext','requestContext','value','services'].forEach(function (key) {inspect(own(value,key),depth+1);});
    }
    for (var count = 0; stack.length && count < 3000 && !found; count++) {
      var fiber = stack.pop(); if (!object(fiber) || fibers.has(fiber)) continue; fibers.add(fiber);
      inspect(own(fiber,'memoizedProps'),0); inspect(own(fiber,'pendingProps'),0);
      stack.push(own(fiber,'child'),own(fiber,'sibling'),own(fiber,'current'),own(own(fiber,'stateNode'),'current'));
    }
    return found;
  }
  function validNative(value) {
    var keys = ['enabled','known','screensaver','locked','sleeping','session_active'];
    return object(value) && Object.keys(value).length === keys.length && keys.every(function (key) {return typeof own(value,key)==='boolean';});
  }
  function mapState(state, native) {
    var inactive = native.screensaver || native.locked || native.sleeping || !native.session_active || state.suspendedState === 'Suspending';
    return Object.assign({}, state, {machineIdleState: inactive ? 'Idle' : 'Active', machineLockState: native.locked ? 'Locked' : 'Active'});
  }
  function createAdapter(options) {
    var binding = null, native = null, timer = null, attempts = 0;
    var stats = {enabled: false, native_known: false, host_found: false, hooked: false, mapped_idle: false, updates: 0, unsupported: false};
    function stopTimer() {if (timer !== null) options.clearTimer(timer); timer = null;}
    function detach(restoreState) {
      if (!binding) return;
      var saved = binding; binding = null; saved.active = false;
      try {
        if (own(saved.subject, 'next') === saved.wrapper) {
          if (saved.descriptor) Object.defineProperty(saved.subject, 'next', saved.descriptor);
          else delete saved.subject.next;
          if (restoreState && validState(saved.lastOriginal)) Reflect.apply(saved.next, saved.subject, [saved.lastOriginal]);
        }
      } catch (_) {stats.unsupported = true;}
      stats.hooked = false;
    }
    function transformed(value) {return native && native.enabled && native.known ? mapState(value, native) : value;}
    function apply() {
      if (!binding || !native || !native.enabled || !native.known) return;
      if (own(binding.subject, 'next') !== binding.wrapper) {detach(false); stats.unsupported = true; return;}
      try {
        if (own(binding.subject,'closed') === true || own(binding.subject,'isStopped') === true) throw new Error('Provider stopped');
        var current = Reflect.apply(binding.get, binding.subject, []);
        if (!validState(current)) throw new Error('Unsupported provider state');
        var mapped = transformed(binding.lastOriginal);
        stats.mapped_idle = mapped.machineIdleState === 'Idle';
        if (stateKeys.some(function (key) {return current[key] !== mapped[key];})) {
          Reflect.apply(binding.next, binding.subject, [mapped]); stats.updates++;
        }
      } catch (_) {detach(false); stats.unsupported = true;}
    }
    function attach(host) {
      if (!compatible(host)) return false;
      var subject = own(host, '_hostStateSubject'), descriptor = Object.getOwnPropertyDescriptor(subject,'next');
      if (descriptor ? !descriptor.configurable && !descriptor.writable : !Object.isExtensible(subject)) return false;
      var saved = {subject: subject, get: method(subject,'getValue'), next: method(subject,'next'), descriptor: descriptor, active: true};
      saved.lastOriginal = Reflect.apply(saved.get, subject, []);
      saved.wrapper = function (value) {
        if (!saved.active) return Reflect.apply(saved.next, this, [value]);
        if (!validState(value)) {
          detach(false); stats.unsupported = true;
          return Reflect.apply(saved.next, this, [value]);
        }
        saved.lastOriginal = value;
        return Reflect.apply(saved.next, this, [transformed(value)]);
      };
      try {Object.defineProperty(subject,'next',Object.assign({},descriptor || {configurable:true,enumerable:false,writable:true},{value:saved.wrapper}));}
      catch (_) {return false;}
      binding = saved; stats.host_found = true; stats.hooked = true; return true;
    }
    function scan() {
      if (!native || !native.enabled || !native.known || binding) return;
      attempts++;
      try {var host = options.findHost(); if (host) attach(host);} catch (_) {stats.unsupported = true;}
      if (binding || attempts >= 60) stopTimer();
      apply();
    }
    function receive(value) {
      if (!validNative(value)) return;
      var wasEnabled = stats.enabled;
      native = Object.assign({}, value); stats.enabled = value.enabled; stats.native_known = value.known;
      if (!value.enabled || !value.known) {stopTimer(); detach(true); return;}
      if (!wasEnabled) attempts = 0;
      scan(); apply();
      if (!binding && attempts < 60 && timer === null) timer = options.setTimer(scan, 2000);
    }
    return {receive:receive, getStats:function () {return Object.assign({},stats);}, dispose:function () {stopTimer(); detach(true);}};
  }
  return {createAdapter:createAdapter, discover:discover, compatible:compatible};
});
