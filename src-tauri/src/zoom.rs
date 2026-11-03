//! Per-app Teams page zoom. Native menus do not grant remote page permissions;
//! the small keyboard fallback accepts only enumerated actions on Teams origins.
use std::{path::PathBuf, sync::Mutex};
use tauri::{
    menu::{MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
    AppHandle, Manager, Runtime, WebviewWindow,
};

const MIN: u16 = 50;
const MAX: u16 = 200;
const STEP: u16 = 10;
#[derive(Clone, Copy)]
pub enum Action {
    In,
    Out,
    Reset,
}
impl Action {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in" => Some(Self::In),
            "out" => Some(Self::Out),
            "reset" => Some(Self::Reset),
            _ => None,
        }
    }
}
fn next(current: u16, action: Action) -> u16 {
    match action {
        Action::In => current.saturating_add(STEP).min(MAX),
        Action::Out => current.saturating_sub(STEP).max(MIN),
        Action::Reset => 100,
    }
}

pub struct Zoom {
    percent: Mutex<u16>,
    preference: PathBuf,
}
impl Zoom {
    fn new(data: PathBuf) -> Self {
        let preference = data.join("zoom.json");
        let percent = std::fs::read(&preference)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<u16>(&bytes).ok())
            .filter(|value| (MIN..=MAX).contains(value) && value % STEP == 0)
            .unwrap_or(100);
        Self {
            percent: Mutex::new(percent),
            preference,
        }
    }
    pub fn apply<R: Runtime>(
        &self,
        window: &WebviewWindow<R>,
        action: Action,
    ) -> Result<(), String> {
        let mut current = self.percent.lock().map_err(|_| "Zoom state unavailable")?;
        let updated = next(*current, action);
        window
            .set_zoom(f64::from(updated) / 100.0)
            .map_err(|_| "Cannot change page zoom")?;
        *current = updated;
        let temporary = self.preference.with_extension("json.tmp");
        std::fs::write(&temporary, updated.to_string())
            .and_then(|()| std::fs::rename(temporary, &self.preference))
            .map_err(|_| "Zoom changed, but its preference could not be saved".into())
    }
    pub fn restore<R: Runtime>(&self, window: &WebviewWindow<R>) {
        if let Ok(value) = self.percent.lock() {
            let _ = window.set_zoom(f64::from(*value) / 100.0);
        }
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>, data: PathBuf) -> tauri::Result<()> {
    app.manage(Zoom::new(data));
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    let mut view = None;
    for item in menu.items()? {
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
            menu.append(&view)?;
            view
        }
    };
    view.append_items(&[
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(
            app,
            "teams-zoom-in",
            "Zoom In",
            true,
            Some("CmdOrCtrl+Equal"),
        )?,
        &MenuItem::with_id(
            app,
            "teams-zoom-out",
            "Zoom Out",
            true,
            Some("CmdOrCtrl+Minus"),
        )?,
        &MenuItem::with_id(
            app,
            "teams-zoom-reset",
            "Actual Size",
            true,
            Some("CmdOrCtrl+0"),
        )?,
    ])?;
    app.on_menu_event(|app, event| {
        let action = match event.id.as_ref() {
            "teams-zoom-in" => Action::In,
            "teams-zoom-out" => Action::Out,
            "teams-zoom-reset" => Action::Reset,
            _ => return,
        };
        if let Some(window) = app.get_webview_window("main") {
            if window.is_focused().unwrap_or(false)
                && window
                    .url()
                    .is_ok_and(|url| super::configuration::current().notification_origin(&url))
            {
                if let Err(error) = app.state::<Zoom>().apply(&window, action) {
                    eprintln!("[msteams] {error}");
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_steps_and_reset_are_deterministic() {
        assert_eq!(next(100, Action::In), 110);
        assert_eq!(next(100, Action::Out), 90);
        assert_eq!(next(MAX, Action::In), MAX);
        assert_eq!(next(MIN, Action::Out), MIN);
        assert_eq!(next(150, Action::Reset), 100);
        assert!(Action::parse("in").is_some());
        assert!(Action::parse("arbitrary").is_none());
    }
    #[test]
    fn invalid_or_missing_saved_levels_default_to_actual_size() {
        let root = std::env::temp_dir().join(format!("teams-zoom-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        for value in ["0", "1000", "105", "not json", "-1"] {
            std::fs::write(root.join("zoom.json"), value).unwrap();
            assert_eq!(*Zoom::new(root.clone()).percent.lock().unwrap(), 100);
        }
        std::fs::write(root.join("zoom.json"), "130").unwrap();
        assert_eq!(*Zoom::new(root.clone()).percent.lock().unwrap(), 130);
        std::fs::remove_dir_all(root).unwrap();
    }
}
