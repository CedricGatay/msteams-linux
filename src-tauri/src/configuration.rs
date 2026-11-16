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

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Application {
    #[default]
    Teams,
    Outlook,
    Word,
    Excel,
    Powerpoint,
}

impl Application {
    fn preset_json(self) -> Option<&'static str> {
        match self {
            Self::Teams => None,
            Self::Outlook => Some(include_str!("../../config/outlook.json")),
            Self::Word => Some(include_str!("../../config/word.json")),
            Self::Excel => Some(include_str!("../../config/excel.json")),
            Self::Powerpoint => Some(include_str!("../../config/powerpoint.json")),
        }
    }

    pub fn remote_permissions(self) -> Vec<&'static str> {
        let mut permissions = vec![
            "allow-show-notification",
            "allow-notification-permission",
            "allow-close-notification",
            "allow-set-unread-count",
            "allow-zoom-teams",
        ];
        if self == Self::Teams {
            permissions.extend([
                "allow-foreground-notifications-enabled",
                "allow-media-permission-status",
                "allow-screen-presence-status",
            ]);
        }
        permissions
    }

    pub fn is_document_app(self) -> bool {
        matches!(self, Self::Word | Self::Excel | Self::Powerpoint)
    }

    pub fn icon_png(self) -> &'static [u8] {
        match self {
            Self::Teams => include_bytes!("../../dist/services/teams.png"),
            Self::Outlook => include_bytes!("../../dist/services/outlook.png"),
            Self::Word => include_bytes!("../../dist/services/word.png"),
            Self::Excel => include_bytes!("../../dist/services/excel.png"),
            Self::Powerpoint => include_bytes!("../../dist/services/powerpoint.png"),
        }
    }

    pub fn icon_icns(self) -> &'static [u8] {
        match self {
            Self::Teams => include_bytes!("../icons/icon.icns"),
            Self::Outlook => include_bytes!("../icons/outlook/icon.icns"),
            Self::Word => include_bytes!("../icons/word/icon.icns"),
            Self::Excel => include_bytes!("../icons/excel/icon.icns"),
            Self::Powerpoint => include_bytes!("../icons/powerpoint/icon.icns"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Teams => "Teams",
            Self::Outlook => "Outlook",
            Self::Word => "Word",
            Self::Excel => "Excel",
            Self::Powerpoint => "PowerPoint",
        }
    }
}

// No Debug: a user configuration can contain private organization details.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub version: u8,
    #[serde(default)]
    pub application: Application,
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

/// Decode only our fixed-size RGBA assets without enabling Tauri's optional
/// image stack (whose latest release requires a newer Rust toolchain).
pub fn icon_image(bytes: &[u8]) -> Result<tauri::image::Image<'static>, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|_| "Invalid built-in PNG icon")?;
    let info = reader.info();
    if info.width != 128
        || info.height != 128
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err("Built-in icons must be 128-pixel RGBA PNGs".into());
    }
    let mut rgba = vec![0; 128 * 128 * 4];
    reader
        .next_frame(&mut rgba)
        .map_err(|_| "Cannot decode built-in PNG icon")?;
    Ok(tauri::image::Image::new_owned(rgba, 128, 128))
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
        let application: Application = object
            .get("application")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| "Application must be teams, outlook, word, excel or powerpoint")?
            .unwrap_or_default();
        if let Some(json) = application.preset_json() {
            let preset: serde_json::Value =
                serde_json::from_str(json).expect("valid application preset");
            for (key, value) in preset.as_object().unwrap() {
                merged
                    .as_object_mut()
                    .unwrap()
                    .insert(key.clone(), value.clone());
            }
        }
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
        let url = Url::parse(&self.teams_url).map_err(|_| "Invalid application URL")?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err(
                "Application URL must use HTTPS without credentials, query or fragment".into(),
            );
        }
        if !self.trusted(&url) {
            return Err("Application URL must belong to a configured trusted host".into());
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

    /// Switching the export target keeps organization authentication settings,
    /// but replaces service-specific defaults. The selected input stays intact.
    pub fn for_application(mut self, application: Application) -> Result<Self, String> {
        if self.application == application {
            return Ok(self);
        }
        let old = Self::parse(
            &serde_json::to_vec(&serde_json::json!({
                "application": self.application
            }))
            .unwrap(),
        )?;
        let preset = Self::parse(
            &serde_json::to_vec(&serde_json::json!({
                "application": application
            }))
            .unwrap(),
        )?;
        self.session_cookie_hosts
            .retain(|host| !old.session_cookie_hosts.contains(host));
        self.session_cookie_hosts
            .extend(preset.session_cookie_hosts);
        // Do not reintroduce the broad built-in navigation suffix allowlist
        // when an organization deliberately restricted its input config. Add
        // only exact HTTPS hosts required by the target service instead.
        for host in std::iter::once(preset.teams_url.as_str())
            .chain(preset.notification_origins.iter().map(String::as_str))
            .filter_map(|value| Url::parse(value).ok())
            .filter_map(|url| url.host_str().map(str::to_owned))
            .chain(self.session_cookie_hosts.iter().cloned())
            .collect::<Vec<_>>()
        {
            let url = Url::parse(&format!("https://{host}/")).expect("validated host");
            if !self.trusted(&url) {
                self.trusted_auth_hosts.push(host);
            }
        }
        self.application = application;
        self.teams_url = preset.teams_url;
        self.notification_origins = preset.notification_origins;
        self.native_foreground = false;
        // Preserve a personalized name, replacing only the built-in name.
        if self.name == old.name {
            self.name = preset.name;
        }
        self.validate()?;
        Ok(self)
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
    /// Office editors open real documents in related windows. Do not treat
    /// those as disposable OAuth windows just because the main window focuses.
    pub fn disposable_popup(&self, url: Option<&Url>) -> bool {
        if !self.application.is_document_app() {
            return true;
        }
        // A related editor may still be bootstrapping on about:blank. Only
        // known authentication hosts are safe to discard automatically.
        let Some(url) = url else {
            return false;
        };
        let Some(host) = url.host_str() else {
            return false;
        };
        self.trusted_auth_hosts.iter().any(|auth| auth == host)
            || host == "login.live.com"
            || host == "microsoftonline.com"
            || host.ends_with(".microsoftonline.com")
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
                "org.9h41.{}.generated.app{}",
                self.application.label().to_ascii_lowercase(),
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
    fn outlook_defaults_overrides_and_legacy_configs_are_valid() {
        let outlook = AppConfig::parse(br#"{"application":"outlook"}"#).unwrap();
        assert!(outlook.application == Application::Outlook);
        assert_eq!(outlook.name, "Microsoft Outlook");
        assert_eq!(outlook.teams_url, "https://outlook.office.com/mail/");
        for origin in [
            "https://outlook.office.com/",
            "https://outlook.office365.com/",
            "https://outlook.live.com/",
        ] {
            assert!(outlook.notification_origin(&Url::parse(origin).unwrap()));
        }
        assert!(
            !outlook.notification_origin(&Url::parse("https://teams.cloud.microsoft/").unwrap())
        );
        assert!(!outlook
            .notification_origin(&Url::parse("https://outlook.office.com.evil.example/").unwrap()));
        assert!(outlook.cookie_host_allowed("outlook.office.com"));
        assert!(!outlook.native_foreground);
        let teams = AppConfig::parse(br#"{}"#).unwrap();
        assert!(teams.application == Application::Teams);
        assert_eq!(teams.teams_url, AppConfig::default().teams_url);
        let custom = AppConfig::parse(br#"{"application":"outlook","name":"Personal Mail","teams_url":"https://outlook.live.com/mail/"}"#).unwrap();
        assert_eq!(custom.name, "Personal Mail");
        assert_eq!(custom.teams_url, "https://outlook.live.com/mail/");
        assert!(AppConfig::parse(br#"{"application":"unknown"}"#).is_err());
        assert!(AppConfig::parse(include_bytes!("../../config/outlook.json")).is_ok());
    }

    #[test]
    fn document_services_have_valid_presets_icons_and_isolated_identities() {
        let mut identities = std::collections::HashSet::new();
        let mut profiles = std::collections::HashSet::new();
        for application in [
            Application::Teams,
            Application::Outlook,
            Application::Word,
            Application::Excel,
            Application::Powerpoint,
        ] {
            let input =
                serde_json::to_vec(&serde_json::json!({"application": application})).unwrap();
            let config = AppConfig::parse(&input).unwrap();
            assert_eq!(config.name, format!("Microsoft {}", application.label()));
            assert!(config.trusted(&Url::parse(&config.teams_url).unwrap()));
            assert!(config.notification_origin(&Url::parse(&config.teams_url).unwrap()));
            assert!(!config.native_foreground);
            let icon = icon_image(application.icon_png()).unwrap();
            assert_eq!((icon.width(), icon.height()), (128, 128));
            assert!(application.icon_icns().starts_with(b"icns"));
            let generated = config.clone().export_identity(None);
            assert!(identities.insert(generated.identifier().to_owned()));
            assert!(profiles.insert(generated.profile_id.clone()));
            let round_trip = AppConfig::parse(&serde_json::to_vec(&generated).unwrap()).unwrap();
            assert!(round_trip.application == application);
            assert_eq!(round_trip.teams_url, config.teams_url);
            for destination in [
                Application::Teams,
                Application::Outlook,
                Application::Word,
                Application::Excel,
                Application::Powerpoint,
            ] {
                let mut personalized = config.clone();
                personalized
                    .trusted_auth_hosts
                    .push("login.example.com".into());
                personalized
                    .session_cookie_hosts
                    .push("login.example.com".into());
                let switched = personalized.for_application(destination).unwrap();
                assert!(switched.application == destination);
                assert!(switched.cookie_host_allowed("login.example.com"));
                assert!(switched.trusted(&Url::parse("https://login.example.com/").unwrap()));
            }
        }
    }

    #[test]
    fn office_document_popups_survive_main_focus_but_auth_popups_remain_disposable() {
        for application in [
            Application::Word,
            Application::Excel,
            Application::Powerpoint,
        ] {
            let mut config = AppConfig::default().for_application(application).unwrap();
            config.trusted_auth_hosts.push("login.example.com".into());
            for target in [
                "https://word-edit.officeapps.live.com/edit",
                "https://example.sharepoint.com/document",
                "https://excel.cloud.microsoft/",
                "https://powerpoint.cloud.microsoft/",
            ] {
                assert!(!config.disposable_popup(Some(&Url::parse(target).unwrap())));
            }
            for target in [
                "https://login.microsoftonline.com/",
                "https://login.live.com/",
                "https://login.example.com/",
            ] {
                assert!(config.disposable_popup(Some(&Url::parse(target).unwrap())));
            }
            assert!(!config.disposable_popup(None));
            assert!(!config.disposable_popup(Some(&Url::parse("about:blank").unwrap())));
        }
        assert!(AppConfig::default()
            .disposable_popup(Some(&Url::parse("https://teams.cloud.microsoft/").unwrap())));
    }

    #[test]
    fn only_teams_receives_presence_and_media_inspection_permissions() {
        for application in [
            Application::Teams,
            Application::Outlook,
            Application::Word,
            Application::Excel,
            Application::Powerpoint,
        ] {
            let permissions = application.remote_permissions();
            assert!(permissions.contains(&"allow-set-unread-count"));
            assert!(permissions.contains(&"allow-zoom-teams"));
            for restricted in [
                "allow-foreground-notifications-enabled",
                "allow-media-permission-status",
                "allow-screen-presence-status",
            ] {
                assert_eq!(
                    permissions.contains(&restricted),
                    application == Application::Teams
                );
            }
        }
    }

    #[test]
    fn switching_services_preserves_restricted_navigation_allowlists() {
        let config = AppConfig::parse(
            br#"{
            "trusted_host_suffixes": ["teams.cloud.microsoft"],
            "trusted_auth_hosts": ["login.example.com"],
            "notification_origins": ["https://teams.cloud.microsoft"],
            "session_cookie_hosts": ["login.example.com", "teams.cloud.microsoft"]
        }"#,
        )
        .unwrap();
        for application in [
            Application::Outlook,
            Application::Word,
            Application::Excel,
            Application::Powerpoint,
        ] {
            let switched = config.clone().for_application(application).unwrap();
            assert_eq!(switched.trusted_host_suffixes, config.trusted_host_suffixes);
            assert!(switched.trusted(&Url::parse(&switched.teams_url).unwrap()));
            assert!(switched.trusted(&Url::parse("https://login.example.com/").unwrap()));
            assert!(!switched.trusted(&Url::parse("https://unrelated.microsoft.com/").unwrap()));
            assert!(!switched.trusted(&Url::parse("https://unrelated.cloud.microsoft/").unwrap()));
            let service_url = Url::parse(&switched.teams_url).unwrap();
            let child = Url::parse(&format!(
                "https://unconfigured.{}/",
                service_url.host_str().unwrap()
            ))
            .unwrap();
            assert!(!switched.trusted(&child));
        }
    }

    #[test]
    fn builtin_icon_decoder_rejects_malformed_and_wrong_size_assets() {
        assert!(icon_image(b"not a PNG").is_err());
        assert!(icon_image(include_bytes!("../icons/generator/icon.png")).is_err());
        assert!(icon_image(include_bytes!("../../dist/generator.png")).is_ok());
    }

    #[test]
    fn switching_service_keeps_organization_auth_and_assigns_isolated_identity() {
        let teams = AppConfig::parse(br#"{"trusted_auth_hosts":["login.example.com"],"session_cookie_hosts":["login.example.com","teams.cloud.microsoft"],"native_foreground":true}"#).unwrap();
        let outlook = teams.clone().for_application(Application::Outlook).unwrap();
        assert_eq!(outlook.name, "Microsoft Outlook");
        assert!(outlook.cookie_host_allowed("login.example.com"));
        assert!(outlook.cookie_host_allowed("outlook.office.com"));
        assert!(!outlook.cookie_host_allowed("teams.cloud.microsoft"));
        assert_eq!(outlook.trusted_auth_hosts, teams.trusted_auth_hosts);
        assert!(!outlook.native_foreground);
        let generated = outlook.clone().export_identity(None);
        assert!(generated
            .identifier()
            .starts_with("org.9h41.outlook.generated."));
        assert_ne!(generated.profile_id, teams.export_identity(None).profile_id);
        let update = outlook.clone().export_identity(Some(&generated));
        assert_eq!(update.profile_id, generated.profile_id);
        let bytes = serde_json::to_vec(&generated).unwrap();
        let loaded = AppConfig::parse(&bytes).unwrap();
        assert_eq!(loaded.teams_url, outlook.teams_url);
        let back = outlook.for_application(Application::Teams).unwrap();
        assert_eq!(back.teams_url, AppConfig::default().teams_url);
        assert!(back.cookie_host_allowed("login.example.com"));
    }

    #[test]
    fn runtime_identity_must_match_bundle_and_output_cannot_be_inside_checkout() {
        assert!(AppConfig::default()
            .runtime_identity("com.example.other")
            .is_err());
        assert!(ensure_private_destination(&checkout_root().join("private.app")).is_err());
    }
}
