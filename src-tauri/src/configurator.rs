//! Local-only macOS configuration screen and native file panels.
use crate::{
    app_export::{self, ExportedApp},
    configuration::{self, AppConfig},
};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::{MainThreadMarker, NSArray, NSBundle, NSString, NSURL};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub struct Launch {
    pub configurator: bool,
    pub error: Option<String>,
}
struct Selected {
    config: AppConfig,
}
pub struct ConfiguratorState {
    selected: Mutex<Option<Selected>>,
    output: Mutex<Option<PathBuf>>,
    busy: AtomicBool,
    startup_error: Option<String>,
}

fn resources() -> Option<PathBuf> {
    NSBundle::mainBundle()
        .resourcePath()
        .map(|p| PathBuf::from(p.to_string()))
}
fn bundle() -> PathBuf {
    PathBuf::from(NSBundle::mainBundle().bundlePath().to_string())
}

/// Resolve settings before Tauri derives paths/capabilities/identity from Context.
pub fn prepare(context: &mut tauri::Context<tauri::Wry>) -> Launch {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let picker = args.iter().any(|arg| arg == "--configure")
        || resources().is_some_and(|p| p.join(configuration::CONFIGURATOR_MARKER).is_file());
    let config_arg = args.iter().position(|arg| arg == "--config");
    let actual_id = NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_else(|| configuration::DEFAULT_IDENTIFIER.into());
    if picker {
        context.config_mut().identifier = actual_id;
        context.config_mut().product_name = Some("Teams Configurator".into());
        return Launch {
            configurator: true,
            error: None,
        };
    }
    let loaded = (|| {
        let explicit = config_arg
            .map(|index| {
                args.get(index + 1)
                    .map(PathBuf::from)
                    .ok_or("--config requires a JSON file")
            })
            .transpose()?;
        let embedded = resources()
            .map(|p| p.join(configuration::CONFIG_RESOURCE))
            .filter(|p| p.is_file());
        let private = std::env::var_os("HOME").map(PathBuf::from).map(|p| {
            p.join("Library/Application Support")
                .join(&actual_id)
                .join(configuration::CONFIG_RESOURCE)
        });
        let config = match explicit
            .or(embedded)
            .or_else(|| private.filter(|p| p.is_file()))
        {
            Some(path) => AppConfig::read(&path)?,
            None => AppConfig {
                identifier: None,
                ..AppConfig::default()
            },
        }
        .runtime_identity(&actual_id)?;
        context.config_mut().identifier = config.identifier().into();
        context.config_mut().product_name = Some(config.name.clone());
        context.package_info_mut().name = config.name.clone();
        configuration::initialize(config)
    })();
    match loaded {
        Ok(()) => Launch {
            configurator: false,
            error: None,
        },
        Err(error) => {
            context.config_mut().identifier = actual_id;
            context.config_mut().product_name = Some("Teams Configurator".into());
            Launch {
                configurator: true,
                error: Some(error),
            }
        }
    }
}

pub fn setup(app: &mut tauri::App, error: Option<String>) -> tauri::Result<()> {
    app.manage(ConfiguratorState {
        selected: Mutex::new(None),
        output: Mutex::new(None),
        busy: AtomicBool::new(false),
        startup_error: error,
    });
    crate::session_restore::install_quit_menu(app.handle())?;
    WebviewWindowBuilder::new(
        app,
        "configurator",
        WebviewUrl::App("configurator.html".into()),
    )
    .title("Teams Configurator")
    .inner_size(700.0, 620.0)
    .min_inner_size(560.0, 480.0)
    .center()
    .on_navigation(|url| url.scheme() == "tauri" && url.host_str() == Some("localhost"))
    .build()?;
    Ok(())
}

fn local(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "configurator"
        || !window.url().is_ok_and(|url| {
            url.scheme() == "tauri"
                && url.host_str() == Some("localhost")
                && url.path() == "/configurator.html"
        })
    {
        return Err(
            "Configurator commands are available only in the local configuration screen".into(),
        );
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Selection {
    name: String,
    trusted_auth_host_count: usize,
    teams_url: String,
}

async fn native_panel(
    app: &AppHandle,
    save_name: Option<String>,
) -> Result<Option<PathBuf>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let selected = (|| {
            let mtm = MainThreadMarker::new()?;
            #[allow(deprecated)] // macOS 11 compatibility; file types are fixed, not user code.
            if let Some(name) = save_name {
                let panel = NSSavePanel::savePanel(mtm);
                panel.setTitle(Some(&NSString::from_str("Create standalone Teams app")));
                panel.setCanCreateDirectories(true);
                panel.setNameFieldStringValue(&NSString::from_str(&format!("{name}.app")));
                panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[&*NSString::from_str(
                    "app",
                )])));
                if let Some(home) = std::env::var_os("HOME") {
                    let applications = PathBuf::from(home).join("Applications");
                    if std::fs::create_dir_all(&applications).is_ok() {
                        let url = NSURL::fileURLWithPath(&NSString::from_str(
                            &applications.to_string_lossy(),
                        ));
                        panel.setDirectoryURL(Some(&url));
                    }
                }
                if panel.runModal() != NSModalResponseOK {
                    return None;
                }
                panel.URL()?.path().map(|p| PathBuf::from(p.to_string()))
            } else {
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setTitle(Some(&NSString::from_str("Open configuration file")));
                panel.setCanChooseDirectories(false);
                panel.setCanChooseFiles(true);
                panel.setAllowsMultipleSelection(false);
                panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[&*NSString::from_str(
                    "json",
                )])));
                if panel.runModal() != NSModalResponseOK {
                    return None;
                }
                panel.URL()?.path().map(|p| PathBuf::from(p.to_string()))
            }
        })();
        let _ = tx.send(selected);
    })
    .map_err(|_| "Cannot open native file panel")?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv()
            .map_err(|_| "File panel closed unexpectedly".to_owned())
    })
    .await
    .map_err(|_| "File panel operation failed".to_owned())?
}

fn template() -> Result<PathBuf, String> {
    if let Some(resources) = resources() {
        let included = resources.join("RuntimeTemplate.app");
        if included.is_dir() {
            return Ok(included);
        }
        if resources.join(configuration::CONFIGURATOR_MARKER).exists() {
            return Err("Configurator is missing its prebuilt runtime template".into());
        }
    }
    let current = bundle();
    if current.extension().and_then(|v| v.to_str()) != Some("app") {
        return Err("Build the app bundle first; exports require a prebuilt runtime .app".into());
    }
    Ok(current)
}

#[tauri::command]
pub fn configurator_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<String>, String> {
    local(&window)?;
    Ok(app.state::<ConfiguratorState>().startup_error.clone())
}

#[tauri::command]
pub async fn configurator_open_file(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<Selection>, String> {
    local(&window)?;
    let state = app.state::<ConfiguratorState>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Another configuration operation is in progress".into());
    }
    let result = async {
        *state
            .selected
            .lock()
            .map_err(|_| "Configuration selection unavailable")? = None;
        *state
            .output
            .lock()
            .map_err(|_| "Export state unavailable")? = None;
        let Some(path) = native_panel(&app, None).await? else {
            return Ok(None);
        };
        let config = AppConfig::read(&path)?;
        let selection = Selection {
            name: config.name.clone(),
            teams_url: config.teams_url.clone(),
            trusted_auth_host_count: config.trusted_auth_hosts.len(),
        };
        *state
            .selected
            .lock()
            .map_err(|_| "Configuration selection unavailable")? = Some(Selected { config });
        Ok(Some(selection))
    }
    .await;
    state.busy.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
pub async fn configurator_export(
    app: AppHandle,
    window: WebviewWindow,
    name: String,
) -> Result<Option<ExportedApp>, String> {
    local(&window)?;
    let state = app.state::<ConfiguratorState>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Another configuration operation is in progress".into());
    }
    let result = async {
        let mut config = state
            .selected
            .lock()
            .map_err(|_| "Configuration selection unavailable")?
            .as_ref()
            .ok_or("Open a valid configuration first")?
            .config
            .clone();
        config.name = name;
        config.validate()?;
        let Some(destination) = native_panel(&app, Some(config.name.clone())).await? else {
            return Ok(None);
        };
        let template = template()?;
        let result = tauri::async_runtime::spawn_blocking(move || {
            app_export::export(config, &template, &destination)
        })
        .await
        .map_err(|_| "Application export operation failed".to_owned())??;
        *state
            .output
            .lock()
            .map_err(|_| "Export state unavailable")? = Some(PathBuf::from(&result.path));
        Ok(Some(result))
    }
    .await;
    state.busy.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
pub fn configurator_open_generated(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    local(&window)?;
    let path = app
        .state::<ConfiguratorState>()
        .output
        .lock()
        .map_err(|_| "Export state unavailable")?
        .clone()
        .ok_or("Generate an app first")?;
    std::process::Command::new("/usr/bin/open")
        .arg("-a")
        .arg(path)
        .spawn()
        .map_err(|_| "Cannot launch generated app")?;
    Ok(())
}
