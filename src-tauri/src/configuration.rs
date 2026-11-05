//! Runtime settings are data, not code. Organization configuration belongs in
//! private app storage or an exported bundle outside the source checkout.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::Url;

pub const CONFIG_RESOURCE: &str = "runtime-config.json";
pub const CONFIGURATOR_MARKER: &str = "configurator-mode.json";
pub const DEFAULT_IDENTIFIER: &str = "org.9h41.teams";
pub const LEGACY_PROFILE: &str = "d0d7ad45-528b-4100-93eb-70b0c3bd13d0";
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
const DEFAULT_JSON: &str = include_str!("../../config/default.json");
static CURRENT: OnceLock<AppConfig> = OnceLock::new();

// No Debug: a user configuration can contain private organization details.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub version: u8,
    pub name: String,
    pub teams_url: String,
    pub user_agent: String,
    pub trusted_host_suffixes: Vec<String>,
    pub trusted_auth_hosts: Vec<String>,
    pub notification_origins: Vec<String>,
    pub session_cookie_hosts: Vec<String>,
    #[serde(default)]
    pub restore_session: bool,
    #[serde(default)]
    pub native_foreground: bool,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub generated: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        serde_json::from_str(DEFAULT_JSON).expect("valid generic built-in settings")
    }
}

fn default_browser_user_agent() -> String {
    #[cfg(target_os = "macos")]
    {
        let version = [
            "/Applications/Safari.app/Contents/Info.plist",
            "/System/Applications/Safari.app/Contents/Info.plist",
        ]
        .iter()
        .find_map(|path| {
            let value = plist::Value::from_file(path).ok()?;
            let version = value
                .as_dictionary()?
                .get("CFBundleShortVersionString")?
                .as_string()?;
            valid_safari_version(version).then(|| version.to_owned())
        })
        .unwrap_or_else(|| "17.1".into());
        format!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{version} Safari/605.1.15")
    }
    #[cfg(not(target_os = "macos"))]
    {
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36".into()
    }
}

#[cfg(any(target_os = "macos", test))]
fn valid_safari_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 16
        && version.split('.').all(|part| {
            !part.is_empty() && part.len() <= 3 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub fn current() -> &'static AppConfig {
    CURRENT.get_or_init(AppConfig::default)
}
pub fn initialize(config: AppConfig) -> Result<(), String> {
    CURRENT
        .set(config)
        .map_err(|_| "Runtime configuration was already initialized".into())
}

fn host(input: &str) -> Result<String, String> {
    let normalized = input.to_ascii_lowercase();
    if input.len() > 253
        || !normalized.contains('.')
        || !input.is_ascii()
        || normalized.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err("Host lists require DNS names, without wildcards, URLs, paths or ports".into());
    }
    Ok(normalized)
}

pub fn valid_identifier(value: &str) -> bool {
    value.len() <= 180
        && value.split('.').count() >= 3
        && value.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

impl AppConfig {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err("Configuration exceeds 64 KiB".into());
        }
        let input: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| "Configuration is not valid JSON")?;
        let object = input
            .as_object()
            .ok_or("Configuration must be a JSON object")?;
        let mut merged: serde_json::Value =
            serde_json::from_str(DEFAULT_JSON).expect("valid defaults");
        // Identity is not inherited by files: a generator assigns it on export.
        merged.as_object_mut().unwrap().remove("identifier");
        merged.as_object_mut().unwrap().remove("profile_id");
        for (key, value) in object {
            merged
                .as_object_mut()
                .unwrap()
                .insert(key.clone(), value.clone());
        }
        let mut config: Self = serde_json::from_value(merged)
            .map_err(|_| "Configuration has invalid or unknown fields")?;
        config.validate()?;
        Ok(config)
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|_| "Cannot open configuration file")?;
        let mut bytes = Vec::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read configuration file")?;
        Self::parse(&bytes)
    }

    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported configuration version".into());
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.ends_with(".app")
            || self
                .name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
            || matches!(self.name.as_str(), "." | "..")
        {
            return Err(
                "App name must be a display name of 1–80 characters, without paths or .app suffix"
                    .into(),
            );
        }
        if self.user_agent.is_empty()
            || self.user_agent.len() > 1024
            || self.user_agent.chars().any(char::is_control)
        {
            return Err("Invalid user agent".into());
        }
        for hosts in [
            &mut self.trusted_host_suffixes,
            &mut self.trusted_auth_hosts,
            &mut self.session_cookie_hosts,
        ] {
            if hosts.len() > 128 {
                return Err("Too many configured hosts".into());
            }
            for value in hosts.iter_mut() {
                *value = host(value)?;
            }
            hosts.sort();
            hosts.dedup();
        }
        let url = Url::parse(&self.teams_url).map_err(|_| "Invalid Teams URL")?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err("Teams URL must use HTTPS without credentials, query or fragment".into());
        }
        if !self.trusted(&url) {
            return Err("Teams URL must belong to a configured trusted host".into());
        }
        self.teams_url = url.to_string();
        if self.notification_origins.is_empty() || self.notification_origins.len() > 16 {
            return Err("Configure 1–16 notification origins".into());
        }
        let mut origins = Vec::new();
        for origin in &self.notification_origins {
            let url = Url::parse(origin).map_err(|_| "Invalid notification origin")?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || !self.trusted(&url)
            {
                return Err("Notification origins must be trusted HTTPS origins without paths or credentials".into());
            }
            origins.push(url.origin().ascii_serialization());
        }
        origins.sort();
        origins.dedup();
        self.notification_origins = origins;
        for value in &self.session_cookie_hosts {
            let url =
                Url::parse(&format!("https://{value}/")).map_err(|_| "Invalid cookie host")?;
            if !self.trusted(&url) {
                return Err("Session cookie hosts must also be trusted navigation hosts".into());
            }
        }
        if self
            .identifier
            .as_ref()
            .is_some_and(|id| !valid_identifier(id))
        {
            return Err("Invalid bundle identifier".into());
        }
        if let Some(profile) = &self.profile_id {
            let id = uuid::Uuid::parse_str(profile).map_err(|_| "Invalid profile UUID")?;
            if id.is_nil() {
                return Err("Profile UUID must not be nil".into());
            }
        }
        Ok(())
    }

    /// Explicit overrides remain exact. Automatic macOS identification matches
    /// WebKit/Safari rather than advertising unsupported Chromium behavior.
    pub fn browser_user_agent(&self) -> &str {
        static AUTOMATIC: OnceLock<String> = OnceLock::new();
        if self.user_agent == "auto" {
            AUTOMATIC.get_or_init(default_browser_user_agent)
        } else {
            &self.user_agent
        }
    }

    pub fn identifier(&self) -> &str {
        self.identifier.as_deref().unwrap_or(DEFAULT_IDENTIFIER)
    }
    pub fn profile(&self) -> [u8; 16] {
        *uuid::Uuid::parse_str(self.profile_id.as_deref().unwrap_or(LEGACY_PROFILE))
            .expect("validated profile UUID")
            .as_bytes()
    }
    pub fn trusted(&self, url: &Url) -> bool {
        match url.scheme() {
            "tauri" | "asset" | "ipc" | "about" | "data" | "blob" => return true,
            "http" | "https" => {}
            _ => return false,
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        (url.scheme() == "https" && self.trusted_auth_hosts.iter().any(|value| value == host))
            || self
                .trusted_host_suffixes
                .iter()
                .any(|suffix| host == suffix || host.ends_with(&format!(".{suffix}")))
    }
    pub fn cookie_host_allowed(&self, domain: &str) -> bool {
        let host = domain.strip_prefix('.').unwrap_or(domain);
        self.session_cookie_hosts
            .iter()
            .any(|allowed| allowed == host)
    }
    pub fn notification_origin(&self, url: &Url) -> bool {
        url.scheme() == "https"
            && self
                .notification_origins
                .contains(&url.origin().ascii_serialization())
    }
    pub fn runtime_identity(mut self, bundle_id: &str) -> Result<Self, String> {
        if self.identifier.as_ref().is_some_and(|id| id != bundle_id) {
            return Err(
                "Configuration identity does not match the app bundle; generate a new app instead"
                    .into(),
            );
        }
        self.identifier = Some(bundle_id.into());
        self.profile_id.get_or_insert_with(|| LEGACY_PROFILE.into());
        self.validate()?;
        Ok(self)
    }
    pub fn export_identity(mut self, existing: Option<&Self>) -> Self {
        if let Some(old) = existing {
            self.identifier = old.identifier.clone();
            self.profile_id = old.profile_id.clone();
        } else {
            self.identifier = Some(format!(
                "org.9h41.teams.generated.app{}",
                uuid::Uuid::new_v4().simple()
            ));
            self.profile_id = Some(uuid::Uuid::new_v4().to_string());
        }
        self.generated = true;
        self
    }
}

pub fn checkout_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}
pub fn ensure_private_destination(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or("Output needs a parent directory")?
        .canonicalize()
        .map_err(|_| "Output parent must already exist")?;
    if parent.starts_with(
        checkout_root()
            .canonicalize()
            .unwrap_or_else(|_| checkout_root()),
    ) {
        return Err(
            "Generated apps/configuration must be saved outside the source checkout".into(),
        );
    }
    let name = path.file_name().ok_or("Invalid output name")?;
    Ok(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_browser_identity_matches_platform_and_preserves_overrides() {
        let config = AppConfig::default();
        assert_eq!(config.user_agent, "auto");
        #[cfg(target_os = "macos")]
        {
            assert!(config.browser_user_agent().contains("Version/"));
            assert!(config.browser_user_agent().contains("Safari/605.1.15"));
            assert!(!config.browser_user_agent().contains("Chrome/"));
        }
        #[cfg(not(target_os = "macos"))]
        assert!(config.browser_user_agent().contains("Chrome/"));
        let custom = AppConfig {
            user_agent: "CustomBrowser/1.0".into(),
            ..config
        };
        assert_eq!(custom.browser_user_agent(), "CustomBrowser/1.0");
        for version in ["26.1", "17.1.2", "18"] {
            assert!(valid_safari_version(version));
        }
        for version in [
            "",
            "26..1",
            "26.1\r\nInjected: value",
            "Safari/26",
            "1234.1",
        ] {
            assert!(!valid_safari_version(version));
        }
    }

    #[test]
    fn generic_defaults_and_partial_example_are_valid() {
        let mut defaults = AppConfig::default();
        defaults.validate().unwrap();
        assert!(defaults.restore_session);
        let example = AppConfig::parse(include_bytes!("../../config/example.json")).unwrap();
        assert!(example.restore_session);
        assert!(
            !AppConfig::parse(br#"{"restore_session":false}"#)
                .unwrap()
                .restore_session
        );
        assert!(example.trusted(&Url::parse("https://login.example.com/").unwrap()));
        assert!(!example.trusted(&Url::parse("http://login.example.com/").unwrap()));
        assert!(!example.trusted(&Url::parse("https://login.example.com.evil.example/").unwrap()));
        assert!(example.cookie_host_allowed(".login.example.com"));
        assert!(!example.cookie_host_allowed("sub.login.example.com"));
        assert!(!example.cookie_host_allowed("login.example.com.evil.example"));
    }
    #[test]
    fn rejects_credentials_scripts_wildcards_unknown_fields_and_bad_identity() {
        for input in [
            r#"{"teams_url":"javascript:alert(1)"}"#,
            r#"{"teams_url":"https://user:password@teams.cloud.microsoft/"}"#,
            r#"{"teams_url":"https://teams.cloud.microsoft/?token=synthetic"}"#,
            r#"{"trusted_auth_hosts":["*.example.com"]}"#,
            r#"{"name":"../unsafe"}"#,
            r#"{"password":"synthetic"}"#,
            r#"{"identifier":"bad/id"}"#,
            r#"{"profile_id":"invalid"}"#,
            r#"{"notification_origins":["https://teams.cloud.microsoft/path"]}"#,
        ] {
            assert!(AppConfig::parse(input.as_bytes()).is_err());
        }
    }
    #[test]
    fn new_exports_isolate_and_updates_preserve_identity_without_copying_input_identity() {
        let one = AppConfig::default().export_identity(None);
        let two = AppConfig::default().export_identity(None);
        assert_ne!(one.identifier, two.identifier);
        assert_ne!(one.profile_id, two.profile_id);
        assert_ne!(one.identifier(), DEFAULT_IDENTIFIER);
        assert!(one.identifier().starts_with("org.9h41.teams.generated."));
        let update = AppConfig::default().export_identity(Some(&one));
        assert_eq!(update.identifier, one.identifier);
        assert_eq!(update.profile_id, one.profile_id);
    }
    #[test]
    fn runtime_identity_must_match_bundle_and_output_cannot_be_inside_checkout() {
        assert!(AppConfig::default()
            .runtime_identity("com.example.other")
            .is_err());
        assert!(ensure_private_destination(&checkout_root().join("private.app")).is_err());
    }
}
