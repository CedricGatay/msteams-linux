#[cfg(target_os = "linux")]
mod linux {
    use std::cell::Cell;
    use std::rc::Rc;

    use gtk::prelude::{ContainerExt, GtkWindowExt, WidgetExt};
    use webkit2gtk::{glib, LoadEvent, PermissionRequestExt, SettingsExt, WebView, WebViewExt};

    const TEST: &str = r#"
(async () => {
  let stream;
  let first;
  let second;
  try {
    if (typeof RTCPeerConnection !== 'function') throw new Error('RTCPeerConnection unavailable');
    const videoCodecs = RTCRtpSender.getCapabilities('video').codecs;
    if (!videoCodecs.some(codec => codec.mimeType.toLowerCase() === 'video/h264')) {
      throw new Error('H.264 unavailable: ' + JSON.stringify(videoCodecs));
    }
    if (new URLSearchParams(location.search).has('synthetic')) {
      const canvas = document.createElement('canvas');
      canvas.width = 160;
      canvas.height = 120;
      document.body.appendChild(canvas);
      const context = canvas.getContext('2d');
      let frame = 0;
      setInterval(() => {
        context.fillStyle = frame++ % 2 ? '#146eb4' : '#d83b01';
        context.fillRect(0, 0, canvas.width, canvas.height);
      }, 50);
      const audio = new AudioContext();
      await audio.resume();
      const oscillator = audio.createOscillator();
      const destination = audio.createMediaStreamDestination();
      oscillator.connect(destination);
      oscillator.start();
      stream = new MediaStream([
        ...canvas.captureStream(10).getVideoTracks(),
        ...destination.stream.getAudioTracks()
      ]);
    } else {
      stream = await navigator.mediaDevices.getUserMedia({audio: true, video: true});
    }
    first = new RTCPeerConnection();
    second = new RTCPeerConnection();
    for (const track of stream.getTracks()) first.addTrack(track, stream);
    const firstCandidates = [];
    const secondCandidates = [];
    const playbacks = [];
    const sinks = [];
    first.onicecandidate = event => {
      if (!event.candidate) return;
      if (second.remoteDescription) second.addIceCandidate(event.candidate).catch(reportError);
      else firstCandidates.push(event.candidate);
    };
    second.onicecandidate = event => {
      if (!event.candidate) return;
      if (first.remoteDescription) first.addIceCandidate(event.candidate).catch(reportError);
      else secondCandidates.push(event.candidate);
    };
    second.ontrack = event => {
      const sink = document.createElement(event.track.kind === 'audio' ? 'audio' : 'video');
      sink.muted = true;
      sink.playsInline = true;
      sink.srcObject = event.streams[0];
      document.body.appendChild(sink);
      sinks.push(sink);
      playbacks.push(sink.play());
    };
    const offer = await first.createOffer();
    await first.setLocalDescription(offer);
    await second.setRemoteDescription(offer);
    const answer = await second.createAnswer();
    await second.setLocalDescription(answer);
    await first.setRemoteDescription(answer);
    for (const candidate of firstCandidates) await second.addIceCandidate(candidate);
    for (const candidate of secondCandidates) await first.addIceCandidate(candidate);
    const deadline = Date.now() + 20000;
    while ((first.iceConnectionState !== 'connected' && first.iceConnectionState !== 'completed') && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (first.iceConnectionState !== 'connected' && first.iceConnectionState !== 'completed') {
      throw new Error('ICE did not connect: ' + first.iceConnectionState);
    }
    await Promise.all(playbacks);
    await new Promise(resolve => setTimeout(resolve, 1500));
    const stats = await second.getStats();
    const media = [];
    stats.forEach(report => {
      if (report.type === 'inbound-rtp' && report.bytesReceived > 0) {
        media.push(report.kind + ':' + report.bytesReceived);
      }
    });
    if (!media.some(value => value.startsWith('audio:')) || !media.some(value => value.startsWith('video:'))) {
      throw new Error('No inbound audio/video RTP: ' + JSON.stringify(media));
    }
    const audio = sinks.find(sink => sink.tagName === 'AUDIO');
    const video = sinks.find(sink => sink.tagName === 'VIDEO');
    if (!audio || !video || audio.currentTime <= 0 || video.currentTime <= 0 || video.videoWidth <= 0) {
      throw new Error('Audio/video decoding did not start');
    }
    document.title = 'PASS ' + JSON.stringify({ice: first.iceConnectionState, media, width: video.videoWidth});
  } catch (error) {
    document.title = 'ERROR ' + (error && error.message || error) + '\n' + (error && error.stack || '');
  } finally {
    if (stream) stream.getTracks().forEach(track => track.stop());
    if (first) first.close();
    if (second) second.close();
  }
})();
undefined;
"#;

    pub fn run() {
        gtk::init().expect("GTK initialization failed");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(640, 480);
        let view = WebView::new();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_webrtc(true);
            settings.set_enable_media_stream(true);
            settings.set_enable_media_capabilities(true);
            settings.set_media_playback_requires_user_gesture(false);
            settings.set_enable_write_console_messages_to_stdout(true);
        }
        view.connect_permission_request(|_, request| {
            request.allow();
            true
        });
        window.add(&view);
        window.show_all();
        window.connect_destroy(|_| gtk::main_quit());

        let started = Rc::new(Cell::new(false));
        let started_callback = Rc::clone(&started);
        view.connect_load_changed(move |view, event| {
            if event == LoadEvent::Finished && !started_callback.replace(true) {
                view.evaluate_javascript(TEST, None, None, None::<&gio::Cancellable>, |result| {
                    if let Err(error) = result {
                        eprintln!("JavaScript evaluation failed: {error}");
                        std::process::exit(1);
                    }
                });
            }
        });
        view.connect_title_notify(|view| {
            let Some(title) = view.title() else { return };
            if title.starts_with("PASS ") || title.starts_with("ERROR ") {
                println!("{title}");
                std::process::exit(if title.starts_with("PASS ") { 0 } else { 1 });
            }
        });
        glib::timeout_add_local(std::time::Duration::from_secs(60), || {
            eprintln!("WebRTC smoke test timed out");
            std::process::exit(1);
        });
        let origin = if std::env::var_os("MSTEAMS_WEBRTC_SYNTHETIC").is_some() {
            "http://localhost/?synthetic"
        } else {
            "http://localhost/"
        };
        view.load_html("<title>WebRTC smoke test</title>", Some(origin));
        gtk::main();
    }
}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("WebKitGTK WebRTC smoke test is Linux-only");
}
