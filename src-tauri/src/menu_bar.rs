//! macOS runtime menu controls and per-app tray visibility. No page IPC.
use std::{path::PathBuf, sync::Mutex};
use tauri::{
    menu::{
        CheckMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
    },
    AppHandle, Manager,
};

pub const RELOAD_ID: &str = "reload-teams";
pub const TRAY_ID: &str = "show-tray-icon";

pub struct TrayVisibility {
    current: Mutex<bool>,
    path: PathBuf,
}
impl TrayVisibility {
    pub fn new(directory: PathBuf) -> Self {
        let path = directory.join("tray-visible.json");
        let visible = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
            .unwrap_or(true);
        Self {
            current: Mutex::new(visible),
            path,
        }
    }
    pub fn visible(&self) -> bool {
        self.current.lock().map(|value| *value).unwrap_or(true)
    }
    /// Apply native visibility first; roll it back if persistence fails. A failed
    /// operation must not leave the menu claiming a preference was saved.
    pub fn set_visible(
        &self,
        value: bool,
        apply: impl Fn(bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| "Tray preference unavailable")?;
        apply(value)?;
        if *current == value {
            return Ok(());
        }
        let temporary = self
            .path
            .with_file_name(format!("tray-visible-{}.tmp", uuid::Uuid::new_v4()));
        let saved = (|| -> std::io::Result<()> {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(if value { b"true" } else { b"false" })?;
            std::fs::rename(&temporary, &self.path)
        })();
        if saved.is_err() {
            let _ = std::fs::remove_file(temporary);
            let _ = apply(*current);
            return Err("Could not save tray visibility".into());
        }
        *current = value;
        Ok(())
    }
    pub fn show_fallback(&self) {
        if let Ok(mut current) = self.current.lock() {
            *current = true;
        }
    }
}

/// Move (not duplicate) all former tray commands into the default Help menu.
/// The application menu remains available even when the status item is hidden.
pub fn install<R: tauri::Runtime>(
    app: &AppHandle<R>,
    actions: &Menu<R>,
    directory: PathBuf,
) -> Result<CheckMenuItem<R>, Box<dyn std::error::Error>> {
    app.manage(TrayVisibility::new(directory));
    let root = app
        .menu()
        .ok_or("Application menu must be installed before runtime controls")?;
    let help = match root.get(HELP_SUBMENU_ID) {
        Some(MenuItemKind::Submenu(help)) => help,
        _ => {
            let help = Submenu::with_id(app, HELP_SUBMENU_ID, "Help", true)?;
            root.append(&help)?;
            help
        }
    };
    while let Some(item) = actions.remove_at(0)? {
        help.append(&item)?;
    }
    help.set_as_help_menu_for_nsapp()?;
    let mut view = None;
    for item in root.items()? {
        if let MenuItemKind::Submenu(submenu) = item {
            if submenu.text()? == "View" {
                view = Some(submenu);
                break;
            }
        }
    }
    let view = match view {
        Some(view) => view,
        None => {
            let view = Submenu::with_id(app, "teams-view", "View", true)?;
            root.append(&view)?;
            view
        }
    };
    let visible = CheckMenuItem::with_id(
        app,
        TRAY_ID,
        "Show Tray Icon",
        true,
        app.state::<TrayVisibility>().visible(),
        None::<&str>,
    )?;
    view.append_items(&[
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(app, RELOAD_ID, "Reload Teams", true, Some("CmdOrCtrl+R"))?,
        &visible,
    ])?;
    Ok(visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!("teams-tray-pref-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn default_visible_and_only_explicit_boolean_is_persisted() {
        let path = directory();
        let state = TrayVisibility::new(path.clone());
        assert!(state.visible());
        state.set_visible(false, |_| Ok(())).unwrap();
        assert!(!state.visible());
        assert_eq!(
            std::fs::read_to_string(path.join("tray-visible.json")).unwrap(),
            "false"
        );
        assert!(!TrayVisibility::new(path.clone()).visible());
        std::fs::write(path.join("tray-visible.json"), "not a boolean").unwrap();
        assert!(TrayVisibility::new(path.clone()).visible());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn persistence_failure_rolls_native_visibility_back() {
        let parent = directory();
        let missing = parent.join("missing");
        let state = TrayVisibility::new(missing);
        let applied = RefCell::new(Vec::new());
        assert!(state
            .set_visible(false, |value| {
                applied.borrow_mut().push(value);
                Ok(())
            })
            .is_err());
        assert_eq!(*applied.borrow(), vec![false, true]);
        assert!(state.visible());
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn platform_failure_never_changes_preference() {
        let path = directory();
        let state = TrayVisibility::new(path.clone());
        assert!(state
            .set_visible(false, |_| Err("Native visibility unavailable".into()))
            .is_err());
        assert!(state.visible());
        assert!(!path.join("tray-visible.json").exists());
        std::fs::remove_dir_all(path).unwrap();
    }
}
