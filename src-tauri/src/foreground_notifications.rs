//! Opt-in foreground mirroring stores only a boolean, never notification data.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct ForegroundNotifications {
    enabled: AtomicBool,
    preference: PathBuf,
}

impl ForegroundNotifications {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::load(
            data_dir,
            std::env::var("MSTEAMS_NATIVE_FOREGROUND").as_deref() == Ok("1"),
        )
    }

    fn load(data_dir: PathBuf, force_enabled: bool) -> Self {
        let preference = data_dir.join("native-foreground-notifications.json");
        let saved = std::fs::read(&preference)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
            .unwrap_or(false);
        Self {
            enabled: AtomicBool::new(force_enabled || saved),
            preference,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), ()> {
        let temporary = self.preference.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            if enabled {
                b"true".as_slice()
            } else {
                b"false".as_slice()
            },
        )
        .and_then(|()| std::fs::rename(temporary, &self.preference))
        .map_err(|_| ())?;
        self.enabled.store(enabled, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_off_and_only_persists_the_explicit_boolean() {
        let dir = std::env::temp_dir().join(format!(
            "teams-foreground-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = ForegroundNotifications::load(dir.clone(), false);
        assert!(!state.enabled());
        state.set_enabled(true).unwrap();
        assert_eq!(std::fs::read_to_string(&state.preference).unwrap(), "true");
        assert!(ForegroundNotifications::load(dir.clone(), false).enabled());
        state.set_enabled(false).unwrap();
        assert!(!ForegroundNotifications::load(dir.clone(), false).enabled());
        assert!(ForegroundNotifications::load(dir.clone(), true).enabled());
        assert_eq!(std::fs::read_to_string(&state.preference).unwrap(), "false");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
