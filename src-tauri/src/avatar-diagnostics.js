// Manual snapshot only. Never return URLs, names, attributes, headers or pixels.
(function () {
  var allowed = window.__msteamsAllowedNotificationOrigins;
  if (window.top !== window || !Array.isArray(allowed) || !allowed.includes(location.origin)) return null;
  var selector = '.fui-Avatar, .ui-avatar, .ms-Persona, [data-tid*="avatar" i]';
  var result = {
    image_scanned: 0, image_loaded: 0, image_failed: 0, image_pending: 0,
    avatar_containers: 0, avatar_images: 0, avatar_loaded: 0, avatar_failed: 0, avatar_pending: 0,
    failed_avatar_blob: 0, failed_avatar_https: 0, failed_avatar_data: 0, failed_avatar_other: 0,
    loaded_avatar_blob: 0, loaded_avatar_https: 0, loaded_avatar_svg_data: 0,
    loaded_avatar_other_data: 0, loaded_avatar_other: 0, avatar_tiny_image: 0,
    avatar_css_hidden: 0, avatar_css_transparent: 0, avatar_no_box: 0,
    avatar_in_view: 0, avatar_offscreen: 0,
    photo_context_found: false, photo_base_url_present: false,
    photo_cookie_state_known: false, photo_cookie_valid: false,
    media_auth_service_found: false, media_auth_initialized: false, media_auth_setup_in_progress: false,
    media_auth_base_url_present: false, media_auth_cookie_state_known: false, media_auth_cookie_valid: false,
    photo_requests: 0, photo_http_success: 0, photo_http_401: 0, photo_http_403: 0,
    photo_http_404: 0, photo_http_429: 0, photo_http_server_error: 0,
    photo_http_other: 0, photo_http_unknown: 0, resource_timing_available: false,
    service_worker_controlled: false, image_bitmap_available: false, image_decoder_available: false
  };
  result.avatar_containers = Math.min(document.querySelectorAll(selector).length, 100000);
  result.service_worker_controlled = !!(navigator.serviceWorker && navigator.serviceWorker.controller);
  result.image_bitmap_available = typeof window.createImageBitmap === 'function';
  result.image_decoder_available = typeof window.ImageDecoder === 'function';
  var images = document.querySelectorAll('img');
  for (var i = 0; i < Math.min(images.length, 1000); i++) {
    var image = images[i];
    // Empty-src placeholders are not broken photos. Do not inspect alt/title.
    var source = image.currentSrc || image.getAttribute('src');
    if (!source) continue;
    var state = !image.complete ? 'pending' : image.naturalWidth > 0 ? 'loaded' : 'failed';
    result.image_scanned++; result['image_' + state]++;
    if (!image.closest(selector)) continue;
    result.avatar_images++; result['avatar_' + state]++;
    if (state === 'loaded' && (image.naturalWidth <= 1 || image.naturalHeight <= 1)) result.avatar_tiny_image++;
    var hidden = false, transparent = false, ancestor = image;
    for (var depth = 0; ancestor && depth < 8 && typeof window.getComputedStyle === 'function'; depth++) {
      var style = window.getComputedStyle(ancestor);
      if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') hidden = true;
      if (Number.parseFloat(style.opacity) === 0) transparent = true;
      ancestor = ancestor.parentElement;
    }
    if (hidden) result.avatar_css_hidden++;
    if (transparent) result.avatar_css_transparent++;
    if (typeof image.getBoundingClientRect === 'function') {
      var rect = image.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) result.avatar_no_box++;
      else if (rect.bottom > 0 && rect.right > 0 && rect.top < window.innerHeight && rect.left < window.innerWidth) result.avatar_in_view++;
      else result.avatar_offscreen++;
    }
    var scheme = 'other';
    try {
      var protocol = new URL(source, location.href).protocol;
      if (protocol === 'blob:') scheme = 'blob';
      else if (protocol === 'https:') scheme = 'https';
      else if (protocol === 'data:') scheme = 'data';
    } catch (_) {}
    if (state === 'failed') result['failed_avatar_' + scheme]++;
    if (state === 'loaded') {
      if (scheme === 'data') scheme = /^data:image\/svg\+xml[;,]/i.test(source) ? 'svg_data' : 'other_data';
      result['loaded_avatar_' + scheme]++;
    }
  }
  try {
    var entries = performance.getEntriesByType('resource');
    result.resource_timing_available = true;
    for (var j = Math.max(0, entries.length - 1000); j < entries.length; j++) {
      var entry = entries[j];
      if (!['img', 'fetch', 'xmlhttprequest'].includes(entry.initiatorType)) continue;
      var path;
      try { path = new URL(entry.name, location.href).pathname; } catch (_) { continue; }
      if (!/avatar|photo|profilepicture/i.test(path)) continue;
      result.photo_requests++;
      // Missing/zero status is UNKNOWN, not evidence of an HTTP/network failure.
      var status = entry.responseStatus;
      if (typeof status !== 'number' || status === 0) result.photo_http_unknown++;
      else if (status >= 200 && status < 300) result.photo_http_success++;
      else if ([401, 403, 404, 429].includes(status)) result['photo_http_' + status]++;
      else if (status >= 500 && status <= 599) result.photo_http_server_error++;
      else result.photo_http_other++;
    }
  } catch (_) {}
  // Read only two named photo-state keys and documented backing flags. Never
  // enumerate store records, users, messages, credentials or arbitrary getters.
  function object(value) { return value !== null && typeof value === 'object'; }
  function own(value, key) {
    if (!object(value)) return undefined;
    try { var descriptor = Object.getOwnPropertyDescriptor(value, key); return descriptor && descriptor.value; }
    catch (_) { return undefined; }
  }
  function method(value, key) {
    for (var depth = 0; object(value) && depth < 4; depth++) {
      var descriptor;
      try { descriptor = Object.getOwnPropertyDescriptor(value, key); } catch (_) { return null; }
      if (descriptor) return typeof descriptor.value === 'function' ? descriptor.value : null;
      try { value = Object.getPrototypeOf(value); } catch (_) { return null; }
    }
    return null;
  }
  try {
    var container = document.getElementById('app');
    var stack = [], seenFibers = new WeakSet(), seenContexts = new WeakSet();
    if (container) Object.getOwnPropertyNames(container).forEach(function (key) {
      if (key.startsWith('__reactContainer$') || key.startsWith('__reactFiber$')) stack.push(own(container, key));
    });
    var legacy = own(own(container, '_reactRootContainer'), '_internalRoot');
    if (legacy) stack.push(own(legacy, 'current'));
    var fibers = 0, contexts = 0;
    function inspect(value, depth) {
      if (!object(value) || depth > 4 || contexts >= 3000 || seenContexts.has(value)) return;
      seenContexts.add(value); contexts++;
      var store = own(own(value, 'stateManagementServices'), 'userWindowStore');
      var get = method(store, 'get');
      if (get && !result.photo_context_found) {
        result.photo_context_found = true;
        try {
          var base = Reflect.apply(get, store, ['baseImageUrl']);
          var cookie = Reflect.apply(get, store, ['isImageAuthCookieValid']);
          result.photo_base_url_present = typeof base === 'string' && base.length > 0;
          result.photo_cookie_state_known = typeof cookie === 'boolean';
          result.photo_cookie_valid = cookie === true;
        } catch (_) {}
      }
      var service = own(value, 'mediaAuthService');
      if (object(service) && !result.media_auth_service_found) {
        result.media_auth_service_found = true;
        result.media_auth_initialized = own(service, '_isInitialized') === true;
        result.media_auth_setup_in_progress = own(service, '_isSetupImageCookieInProgress') === true;
        var baseValue = own(own(service, '_baseImageUrlSubject'), '_value');
        var cookieValue = own(own(service, '_isImageAuthCookieValidSubject'), '_value');
        result.media_auth_base_url_present = typeof baseValue === 'string' && baseValue.length > 0;
        result.media_auth_cookie_state_known = typeof cookieValue === 'boolean';
        result.media_auth_cookie_valid = cookieValue === true;
      }
      ['coreServices', 'context', 'defaultContext', 'requestContext', 'value', 'services'].forEach(function (key) {
        inspect(own(value, key), depth + 1);
      });
    }
    while (stack.length && fibers < 3000) {
      var fiber = stack.pop();
      if (!object(fiber) || seenFibers.has(fiber)) continue;
      seenFibers.add(fiber); fibers++;
      inspect(own(fiber, 'memoizedProps'), 0); inspect(own(fiber, 'pendingProps'), 0);
      stack.push(own(fiber, 'child'), own(fiber, 'sibling'), own(fiber, 'current'), own(own(fiber, 'stateNode'), 'current'));
    }
  } catch (_) {}
  return result;
})();
