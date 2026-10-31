//! Theme manifests and theme selection for browser frontends.
//!
//! Themes only describe browser chrome (toolbar, tabs, address bar and colors).
//! They never execute code and cannot replace the browser engine or page styles.

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

const ACTIVE_THEME_FILE: &str = ".active-theme";
const MAX_MANIFEST_BYTES: u64 = 32 * 1024;

const BUILTIN_MANIFESTS: &[(&str, &str)] = &[
    ("archia", include_str!("../themes/archia/theme.toml")),
    ("chrome", include_str!("../themes/chrome/theme.toml")),
    ("firefox", include_str!("../themes/firefox/theme.toml")),
    ("edge", include_str!("../themes/edge/theme.toml")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSource {
    BuiltIn,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeLayout {
    Archia,
    Chrome,
    Firefox,
    Edge,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabPosition {
    Top,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeColors {
    pub window_background: String,
    pub toolbar_background: String,
    pub tab_active_background: String,
    pub tab_inactive_background: String,
    pub address_background: String,
    pub address_text: String,
    pub text: String,
    pub muted_text: String,
    pub accent: String,
    pub border: String,
    pub page_background: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub layout: ThemeLayout,
    pub tab_position: TabPosition,
    pub sidebar: bool,
    pub colors: ThemeColors,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserTheme {
    pub manifest: ThemeManifest,
    pub source: ThemeSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeError {
    Io(String),
    ManifestTooLarge,
    InvalidLine(usize),
    InvalidValue(&'static str),
    MissingField(&'static str),
    InvalidId,
    InvalidColor(&'static str),
    DuplicateTheme(String),
    ThemeNotFound(String),
    BuiltInTheme,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "theme I/O error: {message}"),
            Self::ManifestTooLarge => write!(f, "theme manifest exceeds 32 KiB"),
            Self::InvalidLine(line) => write!(f, "invalid theme manifest line {line}"),
            Self::InvalidValue(key) => write!(f, "invalid value for theme field {key}"),
            Self::MissingField(key) => write!(f, "missing theme field {key}"),
            Self::InvalidId => write!(
                f,
                "theme id must contain lowercase ASCII letters, digits and hyphens"
            ),
            Self::InvalidColor(key) => write!(f, "theme color {key} must be #RRGGBB"),
            Self::DuplicateTheme(id) => write!(f, "theme {id} is already installed"),
            Self::ThemeNotFound(id) => write!(f, "theme {id} was not found"),
            Self::BuiltInTheme => write!(f, "built-in themes cannot be removed or replaced"),
        }
    }
}

impl Error for ThemeError {}

impl From<std::io::Error> for ThemeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

fn parse_string(value: &str, key: &'static str) -> Result<String, ThemeError> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(ThemeError::InvalidValue(key));
    }
    let inner = &value[1..value.len() - 1];
    if inner.contains('"') || inner.contains('\\') || inner.chars().any(char::is_control) {
        return Err(ThemeError::InvalidValue(key));
    }
    Ok(inner.to_owned())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn validate_color(value: String, key: &'static str) -> Result<String, ThemeError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(ThemeError::InvalidColor(key));
    }
    Ok(value.to_ascii_uppercase())
}

impl ThemeManifest {
    /// Parse a small, non-executable TOML-like theme format.
    /// Unknown keys are rejected so misspelled settings never silently disappear.
    pub fn parse(input: &str) -> Result<Self, ThemeError> {
        if input.len() > MAX_MANIFEST_BYTES as usize {
            return Err(ThemeError::ManifestTooLarge);
        }

        let mut section = "";
        let mut id = None;
        let mut name = None;
        let mut description = None;
        let mut version = None;
        let mut author = None;
        let mut layout = None;
        let mut tab_position = None;
        let mut sidebar = None;
        let mut colors: [Option<String>; 11] = Default::default();

        for (index, raw_line) in input.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[colors]" {
                section = "colors";
                continue;
            }
            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(ThemeError::InvalidLine(index + 1));
            };
            let key = raw_key.trim();
            let value = raw_value.trim();

            if section == "colors" {
                let slot = match key {
                    "window_background" => 0,
                    "toolbar_background" => 1,
                    "tab_active_background" => 2,
                    "tab_inactive_background" => 3,
                    "address_background" => 4,
                    "address_text" => 5,
                    "text" => 6,
                    "muted_text" => 7,
                    "accent" => 8,
                    "border" => 9,
                    "page_background" => 10,
                    _ => return Err(ThemeError::InvalidValue("colors")),
                };
                if colors[slot].is_some() {
                    return Err(ThemeError::InvalidValue("duplicate color"));
                }
                colors[slot] = Some(validate_color(parse_string(value, "colors")?, "colors")?);
                continue;
            }

            match key {
                "id" if id.is_none() => id = Some(parse_string(value, "id")?),
                "name" if name.is_none() => name = Some(parse_string(value, "name")?),
                "description" if description.is_none() => {
                    description = Some(parse_string(value, "description")?)
                }
                "version" if version.is_none() => version = Some(parse_string(value, "version")?),
                "author" if author.is_none() => author = Some(parse_string(value, "author")?),
                "layout" if layout.is_none() => {
                    layout = Some(match parse_string(value, "layout")?.as_str() {
                        "archia" => ThemeLayout::Archia,
                        "chrome" => ThemeLayout::Chrome,
                        "firefox" => ThemeLayout::Firefox,
                        "edge" => ThemeLayout::Edge,
                        "custom" => ThemeLayout::Custom,
                        _ => return Err(ThemeError::InvalidValue("layout")),
                    });
                }
                "tab_position" if tab_position.is_none() => {
                    tab_position = Some(match parse_string(value, "tab_position")?.as_str() {
                        "top" => TabPosition::Top,
                        "left" => TabPosition::Left,
                        _ => return Err(ThemeError::InvalidValue("tab_position")),
                    });
                }
                "sidebar" if sidebar.is_none() => {
                    sidebar = Some(match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(ThemeError::InvalidValue("sidebar")),
                    });
                }
                _ => return Err(ThemeError::InvalidValue("duplicate or unknown field")),
            }
        }

        let id = id.ok_or(ThemeError::MissingField("id"))?;
        if !valid_id(&id) {
            return Err(ThemeError::InvalidId);
        }
        let color = |index: usize, key| colors[index].clone().ok_or(ThemeError::MissingField(key));
        Ok(Self {
            id,
            name: name.ok_or(ThemeError::MissingField("name"))?,
            description: description.ok_or(ThemeError::MissingField("description"))?,
            version: version.ok_or(ThemeError::MissingField("version"))?,
            author: author.ok_or(ThemeError::MissingField("author"))?,
            layout: layout.ok_or(ThemeError::MissingField("layout"))?,
            tab_position: tab_position.ok_or(ThemeError::MissingField("tab_position"))?,
            sidebar: sidebar.ok_or(ThemeError::MissingField("sidebar"))?,
            colors: ThemeColors {
                window_background: color(0, "colors.window_background")?,
                toolbar_background: color(1, "colors.toolbar_background")?,
                tab_active_background: color(2, "colors.tab_active_background")?,
                tab_inactive_background: color(3, "colors.tab_inactive_background")?,
                address_background: color(4, "colors.address_background")?,
                address_text: color(5, "colors.address_text")?,
                text: color(6, "colors.text")?,
                muted_text: color(7, "colors.muted_text")?,
                accent: color(8, "colors.accent")?,
                border: color(9, "colors.border")?,
                page_background: color(10, "colors.page_background")?,
            },
        })
    }
}

pub struct ThemeManager {
    directory: PathBuf,
    themes: Vec<BrowserTheme>,
    active_id: String,
}

impl ThemeManager {
    /// Load built-in themes and installed theme manifests from a directory.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, ThemeError> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory)?;

        let mut themes = Vec::new();
        for (expected_id, manifest_text) in BUILTIN_MANIFESTS {
            let manifest = ThemeManifest::parse(manifest_text)?;
            if manifest.id != *expected_id {
                return Err(ThemeError::InvalidValue("built-in theme id"));
            }
            themes.push(BrowserTheme {
                manifest,
                source: ThemeSource::BuiltIn,
            });
        }

        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let manifest_path = entry.path().join("theme.toml");
            if !manifest_path.is_file() {
                continue;
            }
            if fs::metadata(&manifest_path)?.len() > MAX_MANIFEST_BYTES {
                return Err(ThemeError::ManifestTooLarge);
            }
            let text = fs::read_to_string(manifest_path)?;
            let manifest = ThemeManifest::parse(&text)?;
            if entry.file_name().to_str() != Some(manifest.id.as_str()) {
                return Err(ThemeError::InvalidValue("theme directory name"));
            }
            if themes.iter().any(|theme| theme.manifest.id == manifest.id) {
                return Err(ThemeError::DuplicateTheme(manifest.id));
            }
            themes.push(BrowserTheme {
                manifest,
                source: ThemeSource::Installed,
            });
        }

        themes.sort_by(|left, right| {
            left.manifest
                .name
                .to_lowercase()
                .cmp(&right.manifest.name.to_lowercase())
        });
        let requested_active = fs::read_to_string(directory.join(ACTIVE_THEME_FILE))
            .unwrap_or_else(|_| "archia".to_owned());
        let active_id = if themes
            .iter()
            .any(|theme| theme.manifest.id == requested_active.trim())
        {
            requested_active.trim().to_owned()
        } else {
            "archia".to_owned()
        };

        Ok(Self {
            directory,
            themes,
            active_id,
        })
    }

    pub fn themes(&self) -> &[BrowserTheme] {
        &self.themes
    }

    pub fn active_theme(&self) -> &BrowserTheme {
        self.themes
            .iter()
            .find(|theme| theme.manifest.id == self.active_id)
            .expect("active theme is always present")
    }

    pub fn active_id(&self) -> &str {
        &self.active_id
    }

    /// Select and persist a theme by id.
    pub fn select(&mut self, id: &str) -> Result<(), ThemeError> {
        if !self.themes.iter().any(|theme| theme.manifest.id == id) {
            return Err(ThemeError::ThemeNotFound(id.to_owned()));
        }
        fs::write(self.directory.join(ACTIVE_THEME_FILE), id)?;
        self.active_id = id.to_owned();
        Ok(())
    }

    /// Install a theme from a manifest string. Manifests contain data only:
    /// no JavaScript, dynamic libraries, scripts or executable plugin hooks.
    pub fn install_manifest(&mut self, input: &str) -> Result<(), ThemeError> {
        let manifest = ThemeManifest::parse(input)?;
        if self
            .themes
            .iter()
            .any(|theme| theme.manifest.id == manifest.id)
        {
            return Err(ThemeError::DuplicateTheme(manifest.id));
        }
        let destination = self.directory.join(&manifest.id);
        fs::create_dir(&destination)?;
        if let Err(error) = fs::write(destination.join("theme.toml"), input) {
            let _ = fs::remove_dir_all(&destination);
            return Err(error.into());
        }
        self.themes.push(BrowserTheme {
            manifest,
            source: ThemeSource::Installed,
        });
        self.themes.sort_by(|left, right| {
            left.manifest
                .name
                .to_lowercase()
                .cmp(&right.manifest.name.to_lowercase())
        });
        Ok(())
    }

    /// Remove an installed theme. Built-in themes are immutable.
    pub fn uninstall(&mut self, id: &str) -> Result<(), ThemeError> {
        let Some(index) = self.themes.iter().position(|theme| theme.manifest.id == id) else {
            return Err(ThemeError::ThemeNotFound(id.to_owned()));
        };
        if self.themes[index].source == ThemeSource::BuiltIn {
            return Err(ThemeError::BuiltInTheme);
        }
        fs::remove_dir_all(self.directory.join(id))?;
        self.themes.remove(index);
        if self.active_id == id {
            self.select("archia")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "archia-browser-themes-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    const TEST_THEME: &str = r##"
id = "community-night"
name = "Community Night"
description = "A community theme used by tests"
version = "1.0.0"
author = "Archia Browser contributors"
layout = "custom"
tab_position = "left"
sidebar = true

[colors]
window_background = "#101010"
toolbar_background = "#202020"
tab_active_background = "#303030"
tab_inactive_background = "#181818"
address_background = "#252525"
address_text = "#FFFFFF"
text = "#EEEEEE"
muted_text = "#AAAAAA"
accent = "#00AAFF"
border = "#404040"
page_background = "#FFFFFF"
"##;

    #[test]
    fn parses_complete_theme_manifest() {
        let manifest = ThemeManifest::parse(TEST_THEME).unwrap();
        assert_eq!(manifest.id, "community-night");
        assert_eq!(manifest.tab_position, TabPosition::Left);
        assert!(manifest.sidebar);
        assert_eq!(manifest.colors.accent, "#00AAFF");
    }

    #[test]
    fn rejects_path_traversal_theme_ids() {
        let invalid = TEST_THEME.replace("community-night", "../outside");
        assert_eq!(ThemeManifest::parse(&invalid), Err(ThemeError::InvalidId));
    }

    #[test]
    fn rejects_invalid_color_values() {
        let invalid = TEST_THEME.replace("#00AAFF", "blue");
        assert_eq!(
            ThemeManifest::parse(&invalid),
            Err(ThemeError::InvalidColor("colors"))
        );
    }

    #[test]
    fn rejects_unknown_manifest_fields() {
        let invalid = TEST_THEME.replace("sidebar = true", "sidebar = true\nrun = \"script.js\"");
        assert_eq!(
            ThemeManifest::parse(&invalid),
            Err(ThemeError::InvalidValue("duplicate or unknown field"))
        );
    }

    #[test]
    fn loads_builtins_and_persists_theme_selection() {
        let directory = temporary_directory();
        let mut manager = ThemeManager::load(&directory).unwrap();
        assert_eq!(manager.themes().len(), 4);
        manager.select("firefox").unwrap();
        drop(manager);
        let reloaded = ThemeManager::load(&directory).unwrap();
        assert_eq!(reloaded.active_id(), "firefox");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn installs_and_uninstalls_community_theme() {
        let directory = temporary_directory();
        let mut manager = ThemeManager::load(&directory).unwrap();
        manager.install_manifest(TEST_THEME).unwrap();
        assert!(manager
            .themes()
            .iter()
            .any(|theme| theme.manifest.id == "community-night"));
        manager.select("community-night").unwrap();
        manager.uninstall("community-night").unwrap();
        assert_eq!(manager.active_id(), "archia");
        assert!(!directory.join("community-night").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn built_in_themes_cannot_be_uninstalled() {
        let directory = temporary_directory();
        let mut manager = ThemeManager::load(&directory).unwrap();
        assert_eq!(manager.uninstall("archia"), Err(ThemeError::BuiltInTheme));
        fs::remove_dir_all(directory).unwrap();
    }
}
