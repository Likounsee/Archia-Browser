//! Shared cross-platform desktop interface for Archia Browser.
//!
//! The window, widgets and browser chrome are rendered by egui on wgpu.
//! Platform-specific windowing and GPU backends are provided by eframe/winit;
//! this module contains no Win32, GDI or Direct2D calls.

use archia_browser::{
    layout::LayoutViewport,
    net::{
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine, ResourceKind},
        DocumentLoader, HttpTransport, Request, Url,
    },
    surface::SoftwareSurface,
    theme::{ThemeManager, ThemeManifest},
    Browser,
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
};

struct BrowserPolicy;

impl RequestPolicyEngine for BrowserPolicy {
    fn decide(&self, request: &Request) -> PolicyDecision {
        match request.url.scheme() {
            "http" | "https" => PolicyDecision::Allow,
            _ => PolicyDecision::Block,
        }
    }
}

type Loader = DocumentLoader<BrowserPolicy, HttpTransport>;

#[derive(Clone, Copy)]
enum PasteAction {
    Navigate,
    NewTab,
    ImportBookmarks,
}

#[derive(Clone, Copy)]
enum ExportFormat {
    Tsv,
    Csv,
    Markdown,
    Html,
}

#[derive(Clone)]
struct SavedPage {
    title: String,
    url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchProvider {
    Google,
    Bing,
    DuckDuckGo,
}

impl SearchProvider {
    fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Bing => "bing",
            Self::DuckDuckGo => "duckduckgo",
        }
    }
    fn search_url(self, query: &str) -> String {
        let query = encode_query(query);
        match self {
            Self::Google => format!("https://www.google.com/search?q={query}"),
            Self::Bing => format!("https://www.bing.com/search?q={query}"),
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={query}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserAgentProfile {
    Archia,
    ChromiumCompatible,
    FirefoxCompatible,
}

impl UserAgentProfile {
    fn id(self) -> &'static str {
        match self {
            Self::Archia => "archia",
            Self::ChromiumCompatible => "chromium",
            Self::FirefoxCompatible => "firefox",
        }
    }
    fn header(self, version: &str) -> String {
        match self {
            Self::Archia => format!("Mozilla/5.0 (compatible; ArchiaBrowser/{version})"),
            Self::ChromiumCompatible => format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 ArchiaBrowser/{version}"),
            Self::FirefoxCompatible => format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0 ArchiaBrowser/{version}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserSettings {
    search_provider: SearchProvider,
    homepage: String,
    restore_session: bool,
    user_agent: UserAgentProfile,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            search_provider: SearchProvider::Google,
            homepage: "https://www.google.com/".to_owned(),
            restore_session: true,
            user_agent: UserAgentProfile::Archia,
        }
    }
}

impl BrowserSettings {
    fn load(path: &std::path::Path) -> Self {
        let mut settings = Self::default();
        let Ok(contents) = std::fs::read_to_string(path) else {
            return settings;
        };
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "search_provider" => {
                    settings.search_provider = match value.trim() {
                        "bing" => SearchProvider::Bing,
                        "duckduckgo" => SearchProvider::DuckDuckGo,
                        _ => SearchProvider::Google,
                    }
                }
                "homepage" => {
                    if let Ok(url) = Url::parse(value.trim()) {
                        if matches!(url.scheme(), "http" | "https") {
                            settings.homepage = url.to_string();
                        }
                    }
                }
                "restore_session" => settings.restore_session = value.trim() != "false",
                "user_agent" => {
                    settings.user_agent = match value.trim() {
                        "chromium" => UserAgentProfile::ChromiumCompatible,
                        "firefox" => UserAgentProfile::FirefoxCompatible,
                        _ => UserAgentProfile::Archia,
                    }
                }
                _ => {}
            }
        }
        settings
    }

    fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let contents = format!(
            "search_provider={}\nhomepage={}\nrestore_session={}\nuser_agent={}\n",
            self.search_provider.id(),
            self.homepage.replace(['\r', '\n'], ""),
            self.restore_session,
            self.user_agent.id()
        );
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, contents).is_ok()
            && std::fs::rename(&temporary, path).is_err()
        {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::rename(temporary, path);
        }
    }
}

const GRAPHITE: egui::Color32 = egui::Color32::from_rgb(19, 22, 27);
const PANEL: egui::Color32 = egui::Color32::from_rgb(25, 29, 35);
const PANEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(34, 39, 47);
const LIME: egui::Color32 = egui::Color32::from_rgb(190, 242, 100);
const TEXT: egui::Color32 = egui::Color32::from_rgb(238, 241, 235);
const MUTED: egui::Color32 = egui::Color32::from_rgb(147, 155, 163);

#[derive(Clone, Copy)]
struct UiPalette {
    window: egui::Color32,
    toolbar: egui::Color32,
    address: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    accent: egui::Color32,
}

impl Default for UiPalette {
    fn default() -> Self {
        Self {
            window: GRAPHITE,
            toolbar: PANEL,
            address: PANEL_LIGHT,
            text: TEXT,
            muted: MUTED,
            accent: LIME,
        }
    }
}

impl UiPalette {
    fn from_manifest(manifest: &ThemeManifest) -> Self {
        let defaults = Self::default();
        Self {
            window: parse_theme_color(&manifest.colors.window_background)
                .unwrap_or(defaults.window),
            toolbar: parse_theme_color(&manifest.colors.toolbar_background)
                .unwrap_or(defaults.toolbar),
            address: parse_theme_color(&manifest.colors.address_background)
                .unwrap_or(defaults.address),
            text: parse_theme_color(&manifest.colors.text).unwrap_or(defaults.text),
            muted: parse_theme_color(&manifest.colors.muted_text).unwrap_or(defaults.muted),
            accent: parse_theme_color(&manifest.colors.accent).unwrap_or(defaults.accent),
        }
    }
}

fn load_inter_font(ctx: &egui::Context) {
    let mut candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Microsoft/Windows/Fonts/Inter.ttc"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local/share/fonts/Inter.ttc"));
        candidates.push(home.join(".fonts/Inter.ttc"));
    }
    candidates.push(PathBuf::from("/usr/share/fonts/truetype/inter/Inter.ttc"));
    candidates.push(PathBuf::from("/usr/local/share/fonts/Inter.ttc"));

    for path in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "archia-inter".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "archia-inter".to_owned());
        ctx.set_fonts(fonts);
        break;
    }
}

fn parse_theme_color(value: &str) -> Option<egui::Color32> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some(egui::Color32::from_rgb(red, green, blue))
}

fn apply_palette(ctx: &egui::Context, palette: UiPalette) {
    let mut style = (*ctx.style()).clone();
    style.visuals.panel_fill = palette.window;
    style.visuals.window_fill = palette.toolbar;
    style.visuals.extreme_bg_color = palette.window;
    style.visuals.faint_bg_color = palette.toolbar;
    style.visuals.selection.bg_fill = palette.accent.gamma_multiply(0.22);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, palette.accent);
    style.visuals.override_text_color = Some(palette.text);
    style.visuals.widgets.inactive.bg_fill = palette.address;
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, palette.text);
    style.visuals.widgets.hovered.bg_fill = palette.accent.gamma_multiply(0.24);
    style.visuals.widgets.active.bg_fill = palette.accent.gamma_multiply(0.32);
    style.visuals.widgets.open.bg_fill = palette.address;
    ctx.set_style(style);
}

struct DownloadMessage {
    result: Result<(PathBuf, usize), String>,
}

struct ArchiaApp {
    browser: Browser,
    loader: Loader,
    tabs: Vec<archia_browser::core::tabs::TabId>,
    address: String,
    home_query: String,
    status: String,
    page_texture: Option<egui::TextureHandle>,
    page_texture_size: egui::Vec2,
    page_dirty: bool,
    last_viewport_width: u32,
    bookmarks: Vec<SavedPage>,
    history_items: Vec<SavedPage>,
    settings: BrowserSettings,
    themes: Option<ThemeManager>,
    palette: UiPalette,
    homepage_edit: String,
    settings_path: std::path::PathBuf,
    session_path: std::path::PathBuf,
    closed_tabs: Vec<String>,
    bookmark_import_open: bool,
    bookmark_import_text: String,
    pending_paste: Option<PasteAction>,
    pending_restore: Vec<String>,
    restore_started: bool,
    download_sender: Sender<DownloadMessage>,
    download_receiver: Receiver<DownloadMessage>,
    focus_address: bool,
}

impl ArchiaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = GRAPHITE;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(12, 14, 18);
        style.visuals.faint_bg_color = PANEL;
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(66, 91, 43);
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, LIME);
        style.visuals.widgets.inactive.bg_fill = PANEL_LIGHT;
        style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 56, 63);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(57, 75, 44);
        style.visuals.widgets.open.bg_fill = PANEL_LIGHT;
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(11.0, 7.0);
        cc.egui_ctx.set_style(style);

        let browser = Browser::new();
        let first_tab = browser
            .active_tab_id()
            .expect("Browser creates an initial tab");
        let loader =
            browser.document_loader(NetworkPipeline::new(BrowserPolicy), HttpTransport::new());
        let data_dir = browser_data_directory();
        let themes = ThemeManager::load(data_dir.join("themes")).ok();
        let palette = themes
            .as_ref()
            .map(|manager| UiPalette::from_manifest(&manager.active_theme().manifest))
            .unwrap_or_default();
        apply_palette(&cc.egui_ctx, palette);
        load_inter_font(&cc.egui_ctx);
        let bookmarks = load_saved_pages(&data_dir.join("bookmarks.tsv"));
        let history_items = load_saved_pages(&data_dir.join("history.tsv"));
        let settings_path = data_dir.join("settings.ini");
        let session_path = data_dir.join("session.tsv");
        let settings = BrowserSettings::load(&settings_path);
        let restored_tabs = if settings.restore_session {
            load_saved_pages(&session_path)
        } else {
            Vec::new()
        };
        let pending_restore = restored_tabs.into_iter().map(|saved| saved.url).collect();
        let (download_sender, download_receiver) = mpsc::channel();

        let mut app = Self {
            browser,
            loader,
            tabs: vec![first_tab],
            address: String::new(),
            home_query: String::new(),
            status: "Prêt".to_owned(),
            page_texture: None,
            page_texture_size: egui::Vec2::ZERO,
            page_dirty: true,
            last_viewport_width: 0,
            bookmarks,
            history_items,
            homepage_edit: settings.homepage.clone(),
            settings,
            themes,
            palette,
            settings_path,
            session_path,
            closed_tabs: Vec::new(),
            bookmark_import_open: false,
            bookmark_import_text: String::new(),
            pending_paste: None,
            pending_restore,
            restore_started: false,
            download_sender,
            download_receiver,
            focus_address: false,
        };
        app
    }

    fn save_session(&self) {
        if !self.settings.restore_session {
            let _ = std::fs::remove_file(&self.session_path);
            return;
        }
        let entries = self
            .tabs
            .iter()
            .filter_map(|id| {
                let entry = self.browser.tabs().tab(*id)?.history().current()?;
                Some(SavedPage {
                    title: entry.title().unwrap_or_else(|| entry.url().to_string()),
                    url: entry.url().to_string(),
                })
            })
            .collect::<Vec<_>>();
        save_saved_pages(&self.session_path, &entries);
    }

    fn persist_settings(&mut self) {
        if let Ok(url) = Url::parse(&self.settings.homepage) {
            if matches!(url.scheme(), "http" | "https") {
                self.settings.homepage = url.to_string();
            } else {
                self.settings.homepage = BrowserSettings::default().homepage;
            }
        } else {
            self.settings.homepage = BrowserSettings::default().homepage;
        }
        self.settings.save(&self.settings_path);
        self.save_session();
    }

    fn navigate(&mut self, input: &str) {
        let input = input.trim();
        if input.is_empty() {
            return;
        }

        let address = if input.contains("://") {
            input.to_owned()
        } else if input.chars().any(char::is_whitespace) || !looks_like_host(input) {
            self.settings.search_provider.search_url(input)
        } else {
            format!("https://{input}")
        };

        let url = match Url::parse(&address) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => url,
            Ok(_) => {
                self.status = "Seules les adresses HTTP et HTTPS sont autorisées.".to_owned();
                return;
            }
            Err(error) => {
                self.status = format!("Adresse invalide : {error:?}");
                return;
            }
        };

        let mut request = Request::new(url);
        request.headers.insert(
            "accept".into(),
            "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
        );
        request
            .headers
            .insert("accept-language".into(), "fr-FR,fr;q=0.9,en;q=0.8".into());
        request.headers.insert(
            "user-agent".into(),
            self.settings.user_agent.header(self.browser.version()),
        );
        request.policy.resource_kind = ResourceKind::Document;
        request.policy.top_level_navigation = true;

        self.status = format!("Chargement de {}…", request.url);
        let viewport = LayoutViewport::new(self.last_viewport_width.max(960), 760);
        match self.browser.load_request(&self.loader, &request, viewport) {
            Ok(_) => {
                self.address = self
                    .browser
                    .current_url()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                self.status = "Page chargée".to_owned();
                self.record_history();
                self.save_session();
                self.page_dirty = true;
            }
            Err(error) => {
                self.status = format!("Échec du chargement : {error:?}");
            }
        }
    }

    fn new_tab(&mut self) {
        let id = self.browser.new_tab();
        self.tabs.push(id);
        self.address.clear();
        self.page_texture = None;
        self.page_dirty = true;
        self.status = "Nouvel onglet".to_owned();
        self.save_session();
    }

    fn close_tab(&mut self, id: archia_browser::core::tabs::TabId) {
        if self.tabs.len() <= 1 {
            self.status = "Le dernier onglet ne peut pas être fermé.".to_owned();
            return;
        }
        let closed_url = self
            .browser
            .tabs()
            .tab(id)
            .and_then(|tab| tab.history().current())
            .map(|entry| entry.url().to_string());
        if self.browser.close_tab(id) {
            if let Some(url) = closed_url {
                self.closed_tabs.push(url);
                self.closed_tabs.truncate(20);
            }
            self.tabs.retain(|tab| *tab != id);
            self.address = self
                .browser
                .current_url()
                .map(ToString::to_string)
                .unwrap_or_default();
            self.page_texture = None;
            self.page_dirty = true;
            self.status = "Onglet fermé".to_owned();
            self.save_session();
        }
    }

    fn close_tabs_left_of(&mut self, target: archia_browser::core::tabs::TabId) {
        let Some(index) = self.tabs.iter().position(|id| *id == target) else {
            return;
        };
        let ids = self.tabs[..index].to_vec();
        for id in ids {
            self.close_tab(id);
        }
    }

    fn close_tabs_right_of(&mut self, target: archia_browser::core::tabs::TabId) {
        let Some(index) = self.tabs.iter().position(|id| *id == target) else {
            return;
        };
        let ids = self.tabs[index + 1..].to_vec();
        for id in ids {
            self.close_tab(id);
        }
    }

    fn close_duplicate_tabs(&mut self, target: archia_browser::core::tabs::TabId) {
        let target_url = self
            .browser
            .tabs()
            .tab(target)
            .and_then(|tab| tab.history().current())
            .map(|entry| entry.url().to_string());
        let Some(target_url) = target_url else { return };
        let ids = self
            .tabs
            .iter()
            .copied()
            .filter(|id| {
                *id != target
                    && self
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.history().current())
                        .is_some_and(|entry| entry.url().to_string() == target_url)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.close_tab(id);
        }
    }

    fn close_same_site_tabs(&mut self, target: archia_browser::core::tabs::TabId) {
        let host = self
            .browser
            .tabs()
            .tab(target)
            .and_then(|tab| tab.history().current())
            .map(|entry| entry.url().host().to_ascii_lowercase());
        let Some(host) = host else { return };
        let ids = self
            .tabs
            .iter()
            .copied()
            .filter(|id| {
                *id != target
                    && self
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.history().current())
                        .is_some_and(|entry| entry.url().host().eq_ignore_ascii_case(&host))
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.close_tab(id);
        }
    }

    fn reopen_closed_tab(&mut self) {
        if let Some(url) = self.closed_tabs.pop() {
            self.new_tab();
            self.navigate(&url);
            self.status = "Dernier onglet restauré".to_owned();
        } else {
            self.status = "Aucun onglet récemment fermé".to_owned();
        }
    }

    fn open_homepage(&mut self) {
        let homepage = self.settings.homepage.clone();
        self.navigate(&homepage);
    }

    fn copy_open_tabs(&mut self, ctx: &egui::Context, format: ExportFormat) {
        let entries = self
            .tabs
            .iter()
            .filter_map(|id| {
                let entry = self.browser.tabs().tab(*id)?.history().current()?;
                Some((
                    entry.title().unwrap_or_else(|| entry.url().to_string()),
                    entry.url().to_string(),
                ))
            })
            .collect::<Vec<_>>();
        let exported = match format {
            ExportFormat::Tsv => entries
                .iter()
                .map(|(title, url)| format!("{}\t{}", title.replace(['\t', '\r', '\n'], " "), url))
                .collect::<Vec<_>>()
                .join("\n"),
            ExportFormat::Csv => entries
                .iter()
                .map(|(title, url)| format!("{},{}", escape_csv(title), escape_csv(url)))
                .collect::<Vec<_>>()
                .join("\n"),
            ExportFormat::Markdown => entries
                .iter()
                .map(|(title, url)| {
                    format!(
                        "- [{}](<{}>)",
                        title.replace('[', "\\[").replace(']', "\\]"),
                        url
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            ExportFormat::Html => {
                let bookmarks = entries
                    .iter()
                    .map(|(title, url)| SavedPage {
                        title: title.clone(),
                        url: url.clone(),
                    })
                    .collect::<Vec<_>>();
                export_bookmarks_html(&bookmarks)
            }
        };
        ctx.copy_text(exported);
        self.status = "Liste des onglets copiée".to_owned();
    }

    fn copy_history(&mut self, ctx: &egui::Context) {
        let exported = self
            .history_items
            .iter()
            .map(|item| {
                format!(
                    "{}\t{}",
                    item.title.replace(['\t', '\r', '\n'], " "),
                    item.url
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ctx.copy_text(exported);
        self.status = "Historique copié".to_owned();
    }

    fn copy_history_markdown(&mut self, ctx: &egui::Context) {
        let exported = self
            .history_items
            .iter()
            .map(|item| {
                format!(
                    "- [{}](<{}>)",
                    item.title.replace('[', "\\[").replace(']', "\\]"),
                    item.url
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ctx.copy_text(exported);
        self.status = "Historique copié en Markdown".to_owned();
    }

    fn handle_pasted_text(&mut self, text: String) {
        if !is_safe_clipboard_text(&text) {
            self.status = "Presse-papiers vide, trop long ou invalide".to_owned();
            return;
        }
        match self.pending_paste.take() {
            Some(PasteAction::Navigate) => self.navigate(&text),
            Some(PasteAction::NewTab) => {
                self.new_tab();
                self.navigate(&text);
            }
            Some(PasteAction::ImportBookmarks) => {
                self.bookmark_import_text = text;
                self.bookmark_import_open = true;
            }
            None => {}
        }
    }

    fn open_all_bookmarks(&mut self) {
        let urls = self
            .bookmarks
            .iter()
            .map(|item| item.url.clone())
            .take(20)
            .collect::<Vec<_>>();
        for (index, url) in urls.into_iter().enumerate() {
            if index > 0 {
                self.new_tab();
            }
            self.navigate(&url);
        }
    }

    fn start_download(&mut self) {
        let Some(url) = self.browser.current_url().cloned() else {
            self.status = "Aucune page à télécharger".to_owned();
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            self.status = "Seuls les téléchargements HTTP(S) sont autorisés".to_owned();
            return;
        }
        let directory = downloads_directory();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.status = format!("Dossier de téléchargements inaccessible : {error}");
            return;
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pending = directory.join(format!(
            ".archia-pending-{}-{nonce}.tmp",
            std::process::id()
        ));
        let sender = self.download_sender.clone();
        let user_agent = self.settings.user_agent.header(self.browser.version());
        self.status = format!("Téléchargement de {url}…");
        std::thread::spawn(move || {
            let transport = HttpTransport::new().with_limits(64 * 1024 * 1024, 64 * 1024);
            let mut request = Request::new(url);
            request.headers.insert("accept".into(), "*/*".into());
            request.headers.insert("user-agent".into(), user_agent);
            let result = archia_browser::net::download_to_path(&transport, &request, &pending)
                .map_err(|error| format!("{error:?}"))
                .and_then(|download| {
                    let filename = safe_download_filename(&download.suggested_filename);
                    let destination = unique_download_destination(&directory, &filename);
                    std::fs::hard_link(&pending, &destination)
                        .map_err(|error| format!("Impossible de publier le fichier : {error}"))?;
                    let _ = std::fs::remove_file(&pending);
                    Ok((destination, download.bytes_written))
                });
            if result.is_err() {
                let _ = std::fs::remove_file(&pending);
            }
            let _ = sender.send(DownloadMessage { result });
        });
    }

    fn poll_downloads(&mut self) {
        while let Ok(message) = self.download_receiver.try_recv() {
            self.status = match message.result {
                Ok((path, bytes)) => format!(
                    "Téléchargement terminé · {} octets · {}",
                    bytes,
                    path.display()
                ),
                Err(error) => format!("Échec du téléchargement : {error}"),
            };
        }
    }

    fn duplicate_active_tab(&mut self) {
        if let Some(url) = self.browser.current_url().map(ToString::to_string) {
            self.new_tab();
            self.navigate(&url);
        }
    }

    fn go_back(&mut self) {
        if self.browser.history().can_go_back() {
            match self.browser.go_back(
                &self.loader,
                LayoutViewport::new(self.last_viewport_width.max(960), 760),
            ) {
                Ok(_) => {
                    self.sync_address();
                    self.page_dirty = true;
                    self.status = "Page précédente".to_owned();
                }
                Err(error) => self.status = format!("Navigation impossible : {error:?}"),
            }
        }
    }

    fn go_forward(&mut self) {
        if self.browser.history().can_go_forward() {
            match self.browser.go_forward(
                &self.loader,
                LayoutViewport::new(self.last_viewport_width.max(960), 760),
            ) {
                Ok(_) => {
                    self.sync_address();
                    self.page_dirty = true;
                    self.status = "Page suivante".to_owned();
                }
                Err(error) => self.status = format!("Navigation impossible : {error:?}"),
            }
        }
    }

    fn reload(&mut self) {
        let Some(url) = self.browser.current_url().cloned() else {
            return;
        };
        self.navigate(&url.to_string());
    }

    fn sync_address(&mut self) {
        self.address = self
            .browser
            .current_url()
            .map(ToString::to_string)
            .unwrap_or_default();
    }

    fn update_page_texture(&mut self, ctx: &egui::Context, viewport_width: u32) {
        if !self.page_dirty || self.browser.current_page().is_none() {
            return;
        }

        let Some(page) = self.browser.current_page() else {
            self.page_texture = None;
            self.page_dirty = false;
            return;
        };

        let width = viewport_width.clamp(480, 2400);
        let max_height = (64 * 1024 * 1024 / (width as usize * 4)) as u32;
        let height = page.layout.rect.height.max(760).min(max_height.max(760));
        let Some(mut surface) = SoftwareSurface::try_new(width, height) else {
            self.status = "Mémoire insuffisante pour afficher cette page.".to_owned();
            self.page_dirty = false;
            return;
        };

        if self.browser.render_current_page(&mut surface) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                surface.pixels(),
            );
            if let Some(texture) = self.page_texture.as_mut() {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.page_texture =
                    Some(ctx.load_texture("archia-page", image, egui::TextureOptions::LINEAR));
            }
            self.page_texture_size = egui::vec2(width as f32, height as f32);
        }
        self.page_dirty = false;
    }

    fn tab_title(&self, id: archia_browser::core::tabs::TabId) -> String {
        self.browser
            .tabs()
            .tab(id)
            .and_then(|tab| tab.history().current())
            .and_then(|entry| entry.title())
            .map(|title| title.chars().take(24).collect())
            .unwrap_or_else(|| "Nouvel onglet".to_owned())
    }

    fn record_history(&mut self) {
        let Some(url) = self.browser.current_url().map(ToString::to_string) else {
            return;
        };
        let title = self
            .browser
            .current_page()
            .and_then(|page| page.title())
            .unwrap_or_else(|| url.clone());
        self.history_items.retain(|entry| entry.url != url);
        self.history_items.insert(0, SavedPage { title, url });
        self.history_items.truncate(500);
        save_saved_pages(
            &browser_data_directory().join("history.tsv"),
            &self.history_items,
        );
    }

    fn import_bookmarks(&mut self) {
        let imported = parse_bookmark_text(&self.bookmark_import_text);
        let mut added = 0;
        for item in imported {
            if !self.bookmarks.iter().any(|saved| saved.url == item.url) {
                self.bookmarks.push(item);
                added += 1;
            }
        }
        self.bookmarks
            .sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
        save_saved_pages(
            &browser_data_directory().join("bookmarks.tsv"),
            &self.bookmarks,
        );
        self.bookmark_import_text.clear();
        self.bookmark_import_open = false;
        self.status = format!("{added} favori(s) importé(s)");
    }

    fn toggle_bookmark(&mut self) {
        let Some(url) = self.browser.current_url().map(ToString::to_string) else {
            self.status = "Aucune page à ajouter aux favoris.".to_owned();
            return;
        };
        if self.bookmarks.iter().any(|entry| entry.url == url) {
            self.bookmarks.retain(|entry| entry.url != url);
            self.status = "Favori supprimé".to_owned();
        } else {
            let title = self
                .browser
                .current_page()
                .and_then(|page| page.title())
                .unwrap_or_else(|| url.clone());
            self.bookmarks.push(SavedPage { title, url });
            self.status = "Page ajoutée aux favoris".to_owned();
        }
        save_saved_pages(
            &browser_data_directory().join("bookmarks.tsv"),
            &self.bookmarks,
        );
    }

    fn current_links(&self) -> Vec<(String, String)> {
        self.browser
            .current_page()
            .map(|page| {
                page.document
                    .find_elements_by_tag("a")
                    .into_iter()
                    .filter_map(|node| {
                        let href = node.link_href()?.trim();
                        if href.is_empty() {
                            return None;
                        }
                        let label = node.text_content();
                        let label = if label.trim().is_empty() {
                            href.to_owned()
                        } else {
                            label.split_whitespace().collect::<Vec<_>>().join(" ")
                        };
                        Some((label, href.to_owned()))
                    })
                    .take(100)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn open_link(&mut self, href: &str) {
        let Some(base) = self.browser.current_url().cloned() else {
            return;
        };
        match base.resolve(href) {
            Ok(url) => self.navigate(&url.to_string()),
            Err(error) => self.status = format!("Lien invalide : {error:?}"),
        }
    }

    fn link_target_at(&self, x: i32, y: i32) -> Option<String> {
        self.browser
            .current_page()
            .and_then(|page| page.link_at(x, y))
            .and_then(|node| node.link_href().map(str::to_owned))
    }

    fn open_link_at(&mut self, x: i32, y: i32, new_tab: bool) {
        let Some(href) = self.link_target_at(x, y) else {
            return;
        };
        let Some(base) = self.browser.current_url().cloned() else {
            return;
        };
        match base.resolve(&href) {
            Ok(url) => {
                if new_tab {
                    self.new_tab();
                }
                self.navigate(&url.to_string());
            }
            Err(error) => self.status = format!("Lien invalide : {error:?}"),
        }
    }
}

impl eframe::App for ArchiaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_downloads();
        if !self.pending_restore.is_empty() {
            let url = self.pending_restore.remove(0);
            if self.restore_started {
                self.new_tab();
            } else {
                self.restore_started = true;
            }
            self.navigate(&url);
        }
        let shortcuts = ctx.input(|input| {
            let ctrl = input.modifiers.ctrl;
            let shift = input.modifiers.shift;
            let alt = input.modifiers.alt;
            (
                ctrl && input.key_pressed(egui::Key::L),
                ctrl && input.key_pressed(egui::Key::T),
                ctrl && input.key_pressed(egui::Key::W),
                ctrl && input.key_pressed(egui::Key::R),
                ctrl && input.key_pressed(egui::Key::D),
                alt && input.key_pressed(egui::Key::ArrowLeft),
                alt && input.key_pressed(egui::Key::ArrowRight),
                ctrl && shift && input.key_pressed(egui::Key::T),
                ctrl && shift && input.key_pressed(egui::Key::U),
                ctrl && shift && input.key_pressed(egui::Key::Y),
                ctrl && alt && input.key_pressed(egui::Key::D),
                ctrl && alt && shift && input.key_pressed(egui::Key::W),
                ctrl && shift && input.key_pressed(egui::Key::O),
                ctrl && alt && input.key_pressed(egui::Key::C),
                ctrl && alt && input.key_pressed(egui::Key::M),
                alt && input.key_pressed(egui::Key::Home),
                ctrl && shift && input.key_pressed(egui::Key::V),
                ctrl && shift && input.key_pressed(egui::Key::F),
                ctrl && shift && input.key_pressed(egui::Key::M),
                ctrl && alt && shift && input.key_pressed(egui::Key::M),
                ctrl && alt && shift && input.key_pressed(egui::Key::H),
                ctrl && alt && shift && input.key_pressed(egui::Key::U),
                ctrl && alt && shift && input.key_pressed(egui::Key::C),
            )
        });
        let (
            ctrl_l,
            ctrl_t,
            ctrl_w,
            ctrl_r,
            ctrl_d,
            alt_left,
            alt_right,
            reopen_tab,
            export_tabs,
            export_history,
            duplicate_tab,
            close_others,
            open_bookmarks,
            copy_page_tsv,
            copy_page_markdown,
            open_home,
            paste_navigate,
            paste_new_tab,
            paste_bookmarks,
            export_tabs_markdown,
            export_history_markdown,
            export_tabs_html,
            export_tabs_csv,
        ) = shortcuts;
        if ctrl_t {
            self.new_tab();
        }
        if ctrl_w {
            if let Some(id) = self.browser.active_tab_id() {
                self.close_tab(id);
            }
        }
        if ctrl_r {
            self.reload();
        }
        if ctrl_d {
            self.toggle_bookmark();
        }
        if alt_left {
            self.go_back();
        }
        if alt_right {
            self.go_forward();
        }
        if ctrl_l {
            self.focus_address = true;
        }
        if reopen_tab {
            self.reopen_closed_tab();
        }
        if export_tabs {
            self.copy_open_tabs(ctx, ExportFormat::Tsv);
        }
        if export_history {
            self.copy_history(ctx);
        }
        if export_tabs_markdown {
            self.copy_open_tabs(ctx, ExportFormat::Markdown);
        }
        if export_history_markdown {
            self.copy_history_markdown(ctx);
        }
        if export_tabs_html {
            self.copy_open_tabs(ctx, ExportFormat::Html);
        }
        if export_tabs_csv {
            self.copy_open_tabs(ctx, ExportFormat::Csv);
        }
        if paste_navigate || paste_new_tab || paste_bookmarks {
            self.pending_paste = Some(if paste_bookmarks {
                PasteAction::ImportBookmarks
            } else if paste_new_tab {
                PasteAction::NewTab
            } else {
                PasteAction::Navigate
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        let pasted = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Paste(text) = event {
                    Some(text.clone())
                } else {
                    None
                }
            })
        });
        if let Some(text) = pasted {
            self.handle_pasted_text(text);
        }
        if duplicate_tab {
            self.duplicate_active_tab();
        }
        if open_home {
            self.open_homepage();
        }
        if close_others {
            if let Some(active) = self.browser.active_tab_id() {
                let to_close = self
                    .tabs
                    .iter()
                    .copied()
                    .filter(|id| *id != active)
                    .collect::<Vec<_>>();
                for id in to_close {
                    self.close_tab(id);
                }
            }
        }
        if open_bookmarks {
            self.open_all_bookmarks();
        }
        if copy_page_tsv || copy_page_markdown {
            if let Some(url) = self.browser.current_url().map(ToString::to_string) {
                let title = self
                    .browser
                    .current_page()
                    .and_then(|page| page.title())
                    .unwrap_or_else(|| url.clone());
                let text = if copy_page_tsv {
                    format!("{}\t{}", title.replace(['\t', '\r', '\n'], " "), url)
                } else {
                    format!(
                        "[{}](<{}>)",
                        title.replace('[', "\\\\[").replace(']', "\\\\]"),
                        url
                    )
                };
                ctx.copy_text(text);
                self.status = "Lien de la page copié".to_owned();
            }
        }

        egui::TopBottomPanel::top("archia_tabs")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.toolbar)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for id in self.tabs.clone() {
                        let active = self.browser.active_tab_id() == Some(id);
                        let title = self.tab_title(id);
                        ui.horizontal(|ui| {
                            let response = ui.selectable_label(active, title);
                            response.context_menu(|ui| {
                                if ui.button("Fermer cet onglet").clicked() {
                                    self.close_tab(id);
                                }
                                if ui.button("Dupliquer cet onglet").clicked() {
                                    let url = self
                                        .browser
                                        .tabs()
                                        .tab(id)
                                        .and_then(|tab| tab.history().current())
                                        .map(|entry| entry.url().to_string());
                                    if let Some(url) = url {
                                        self.new_tab();
                                        self.navigate(&url);
                                    }
                                }
                                if ui.button("Fermer les onglets à gauche").clicked() {
                                    self.close_tabs_left_of(id);
                                }
                                if ui.button("Fermer les onglets à droite").clicked() {
                                    self.close_tabs_right_of(id);
                                }
                                if ui.button("Fermer les doublons").clicked() {
                                    self.close_duplicate_tabs(id);
                                }
                                if ui.button("Fermer les onglets du même site").clicked() {
                                    self.close_same_site_tabs(id);
                                }
                            });
                            if response.clicked() {
                                self.browser.select_tab(id);
                                self.sync_address();
                                self.page_texture = None;
                                self.page_dirty = true;
                            }
                            if ui.small_button("×").clicked() {
                                self.close_tab(id);
                            }
                        });
                    }
                    if ui.button("＋").on_hover_text("Nouvel onglet").clicked() {
                        self.new_tab();
                    }
                });
            });

        egui::TopBottomPanel::top("archia_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.window)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("←").on_hover_text("Précédent").clicked() {
                        self.go_back();
                    }
                    if ui.button("→").on_hover_text("Suivant").clicked() {
                        self.go_forward();
                    }
                    if ui.button("⟳").on_hover_text("Actualiser").clicked() {
                        self.reload();
                    }
                    let address = ui.add(
                        egui::TextEdit::singleline(&mut self.address)
                            .desired_width(f32::INFINITY)
                            .hint_text("Rechercher ou saisir une adresse"),
                    );
                    if self.focus_address {
                        address.request_focus();
                        self.focus_address = false;
                    }
                    if address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        let target = self.address.clone();
                        self.navigate(&target);
                    }
                    if ui.button("Aller").clicked() {
                        let target = self.address.clone();
                        self.navigate(&target);
                    }
                    let is_bookmarked = self.browser.current_url().is_some_and(|url| {
                        self.bookmarks
                            .iter()
                            .any(|entry| entry.url == url.to_string())
                    });
                    if ui
                        .button(if is_bookmarked { "★" } else { "☆" })
                        .on_hover_text("Ajouter ou retirer des favoris")
                        .clicked()
                    {
                        self.toggle_bookmark();
                    }
                    ui.menu_button("Favoris", |ui| {
                        if ui.button("Ajouter / retirer la page actuelle").clicked() {
                            self.toggle_bookmark();
                        }
                        if ui.button("Importer des favoris…").clicked() {
                            self.bookmark_import_open = true;
                        }
                        if ui.button("Exporter les favoris (TSV)").clicked() {
                            let text = self
                                .bookmarks
                                .iter()
                                .map(|item| format!("{}\t{}", item.title, item.url))
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.ctx().copy_text(text);
                            self.status = "Favoris exportés en TSV".to_owned();
                        }
                        if ui.button("Exporter les favoris (Markdown)").clicked() {
                            let text = self
                                .bookmarks
                                .iter()
                                .map(|item| {
                                    format!(
                                        "- [{}](<{}>)",
                                        item.title.replace('[', "\\[").replace(']', "\\]"),
                                        item.url
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.ctx().copy_text(text);
                            self.status = "Favoris exportés en Markdown".to_owned();
                        }
                        if ui.button("Exporter les favoris (HTML Netscape)").clicked() {
                            ui.ctx().copy_text(export_bookmarks_html(&self.bookmarks));
                            self.status = "Favoris exportés en HTML Netscape".to_owned();
                        }
                        ui.separator();
                        ui.menu_button("Trier", |ui| {
                            if ui.button("Par nom (A → Z)").clicked() {
                                self.bookmarks.sort_by(|a, b| {
                                    a.title.to_lowercase().cmp(&b.title.to_lowercase())
                                });
                                save_saved_pages(
                                    &browser_data_directory().join("bookmarks.tsv"),
                                    &self.bookmarks,
                                );
                            }
                            if ui.button("Par site").clicked() {
                                self.bookmarks.sort_by_key(|item| {
                                    Url::parse(&item.url)
                                        .map(|url| url.host().to_ascii_lowercase())
                                        .unwrap_or_default()
                                });
                                save_saved_pages(
                                    &browser_data_directory().join("bookmarks.tsv"),
                                    &self.bookmarks,
                                );
                            }
                        });
                        ui.separator();
                        let items = self.bookmarks.clone();
                        if items.is_empty() {
                            ui.label("Aucun favori enregistré");
                        }
                        for item in items {
                            if ui.button(&item.title).on_hover_text(&item.url).clicked() {
                                self.navigate(&item.url);
                            }
                        }
                    });
                    ui.menu_button("Historique", |ui| {
                        let items = self.history_items.clone();
                        if items.is_empty() {
                            ui.label("Aucune navigation enregistrée");
                        }
                        for item in items.into_iter().take(30) {
                            if ui.button(&item.title).on_hover_text(&item.url).clicked() {
                                self.navigate(&item.url);
                            }
                        }
                    });
                    ui.menu_button("Liens", |ui| {
                        let links = self.current_links();
                        if links.is_empty() {
                            ui.label("Aucun lien détecté");
                        }
                        for (label, href) in links {
                            if ui.button(label).on_hover_text(&href).clicked() {
                                self.open_link(&href);
                            }
                        }
                    });
                    ui.menu_button("Téléchargements", |ui| {
                        if ui.button("Télécharger la page actuelle").clicked() {
                            self.start_download();
                        }
                        if ui.button("Ouvrir le dossier Téléchargements").clicked() {
                            let directory = downloads_directory();
                            if std::fs::create_dir_all(&directory).is_ok() {
                                open_local_path(&directory);
                            }
                        }
                        ui.separator();
                        let recent = recent_downloads();
                        if recent.is_empty() {
                            ui.label("Aucun téléchargement récent");
                        }
                        for path in recent {
                            let label = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui
                                .button(label.to_string())
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                open_local_path(&path);
                            }
                        }
                    });
                    ui.menu_button("Outils", |ui| {
                        if ui
                            .button("Coller une adresse/recherche · Ctrl+Shift+V")
                            .clicked()
                        {
                            self.pending_paste = Some(PasteAction::Navigate);
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        }
                        if ui
                            .button("Coller dans un nouvel onglet · Ctrl+Shift+F")
                            .clicked()
                        {
                            self.pending_paste = Some(PasteAction::NewTab);
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        }
                        if ui
                            .button("Importer depuis le presse-papiers · Ctrl+Shift+M")
                            .clicked()
                        {
                            self.pending_paste = Some(PasteAction::ImportBookmarks);
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        }
                        if ui.button("Télécharger la page actuelle").clicked() {
                            self.start_download();
                        }
                        if ui
                            .button("Copier les onglets (TSV) · Ctrl+Shift+U")
                            .clicked()
                        {
                            self.copy_open_tabs(ui.ctx(), ExportFormat::Tsv);
                        }
                        if ui.button("Copier les onglets (Markdown)").clicked() {
                            self.copy_open_tabs(ui.ctx(), ExportFormat::Markdown);
                        }
                        if ui.button("Copier les onglets (CSV)").clicked() {
                            self.copy_open_tabs(ui.ctx(), ExportFormat::Csv);
                        }
                        if ui.button("Copier les onglets (HTML Netscape)").clicked() {
                            self.copy_open_tabs(ui.ctx(), ExportFormat::Html);
                        }
                        if ui.button("Copier l'historique · Ctrl+Shift+Y").clicked() {
                            self.copy_history(ui.ctx());
                        }
                        if ui
                            .button("Copier l'historique (Markdown) · Ctrl+Alt+Shift+H")
                            .clicked()
                        {
                            self.copy_history_markdown(ui.ctx());
                        }
                        if ui
                            .button("Rouvrir le dernier onglet fermé · Ctrl+Shift+T")
                            .clicked()
                        {
                            self.reopen_closed_tab();
                        }
                        if ui
                            .button("Ouvrir tous les favoris · Ctrl+Shift+O")
                            .clicked()
                        {
                            self.open_all_bookmarks();
                        }
                    });
                    ui.menu_button("Paramètres", |ui| {
                        ui.label("Moteur de recherche");
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.search_provider,
                                SearchProvider::Google,
                                "Google",
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.search_provider,
                                SearchProvider::Bing,
                                "Bing",
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.search_provider,
                                SearchProvider::DuckDuckGo,
                                "DuckDuckGo",
                            )
                            .clicked();
                        ui.separator();
                        ui.label("Profil User-Agent");
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.user_agent,
                                UserAgentProfile::Archia,
                                "Archia",
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.user_agent,
                                UserAgentProfile::ChromiumCompatible,
                                "Chromium compatible",
                            )
                            .clicked();
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.user_agent,
                                UserAgentProfile::FirefoxCompatible,
                                "Firefox compatible",
                            )
                            .clicked();
                        ui.separator();
                        changed |= ui
                            .checkbox(
                                &mut self.settings.restore_session,
                                "Restaurer la session au démarrage",
                            )
                            .changed();
                        ui.label("Page d'accueil HTTP(S)");
                        ui.text_edit_singleline(&mut self.homepage_edit);
                        if ui.button("Enregistrer la page d'accueil").clicked() {
                            match Url::parse(self.homepage_edit.trim()) {
                                Ok(url) if matches!(url.scheme(), "http" | "https") => {
                                    self.settings.homepage = url.to_string();
                                    self.persist_settings();
                                    self.status = "Page d'accueil enregistrée".to_owned();
                                }
                                _ => {
                                    self.status =
                                        "La page d'accueil doit être une URL HTTP(S) valide"
                                            .to_owned()
                                }
                            }
                        }
                        if ui.button("Ouvrir la page d'accueil · Alt+Home").clicked() {
                            self.open_homepage();
                        }
                        if let Some(manager) = self.themes.as_ref() {
                            ui.separator();
                            ui.label("Thème de l'interface");
                            let choices = manager
                                .themes()
                                .iter()
                                .map(|theme| {
                                    (theme.manifest.id.clone(), theme.manifest.name.clone())
                                })
                                .collect::<Vec<_>>();
                            for (id, name) in choices {
                                let active = self
                                    .themes
                                    .as_ref()
                                    .is_some_and(|themes| themes.active_id() == id);
                                if ui.selectable_label(active, name).clicked() {
                                    if let Some(themes) = self.themes.as_mut() {
                                        if themes.select(&id).is_ok() {
                                            self.palette = UiPalette::from_manifest(
                                                &themes.active_theme().manifest,
                                            );
                                            apply_palette(ui.ctx(), self.palette);
                                            self.status = format!("Thème appliqué : {id}");
                                        }
                                    }
                                }
                            }
                        }
                        if changed {
                            self.persist_settings();
                        }
                    });
                });
            });

        egui::TopBottomPanel::bottom("archia_status")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.toolbar)
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (security_label, security_color) =
                        match self.browser.current_url().map(Url::scheme) {
                            Some("https") => ("HTTPS", self.palette.accent),
                            Some("http") => ("HTTP", egui::Color32::from_rgb(235, 154, 92)),
                            _ => ("LOCAL", self.palette.muted),
                        };
                    ui.label(
                        egui::RichText::new(security_label)
                            .strong()
                            .small()
                            .color(security_color),
                    );
                    ui.label(egui::RichText::new("●").color(self.palette.accent));
                    ui.label(
                        egui::RichText::new(&self.status)
                            .color(self.palette.muted)
                            .small(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "ARCHIA BROWSER · {}",
                                self.browser.version()
                            ))
                            .color(self.palette.muted)
                            .small(),
                        );
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.palette.window).inner_margin(0))
            .show(ctx, |ui| {
                let width = ui.available_width().max(480.0) as u32;
                if width != self.last_viewport_width {
                    self.last_viewport_width = width;
                    if self.browser.current_page().is_some() {
                        self.browser
                            .relayout_current_page(LayoutViewport::new(width, 760));
                        self.page_dirty = true;
                    }
                }

                if self.browser.current_page().is_some() {
                    self.update_page_texture(ctx, width);
                    if let Some(texture) = self.page_texture.as_ref() {
                        let texture_id = texture.id();
                        let image_size = self.page_texture_size;
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let response = ui.add(
                                    egui::Image::new((texture_id, image_size))
                                        .sense(egui::Sense::click_and_drag()),
                                );
                                let response = if let Some(pointer) = response.hover_pos() {
                                    let x = (pointer.x - response.rect.min.x).floor() as i32;
                                    let y = (pointer.y - response.rect.min.y).floor() as i32;
                                    if self.link_target_at(x, y).is_some() {
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand)
                                    } else {
                                        response
                                    }
                                } else {
                                    response
                                };
                                if response.clicked() {
                                    if let Some(pointer) = response.interact_pointer_pos() {
                                        let x = (pointer.x - response.rect.min.x).floor() as i32;
                                        let y = (pointer.y - response.rect.min.y).floor() as i32;
                                        let new_tab = ctx.input(|input| input.modifiers.ctrl);
                                        self.open_link_at(x, y, new_tab);
                                    }
                                }
                            });
                    }
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.20).max(48.0));
                        ui.label(
                            egui::RichText::new("ARCHIA")
                                .size(18.0)
                                .strong()
                                .color(self.palette.accent),
                        );
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("Le web, à votre façon.")
                                .size(34.0)
                                .strong()
                                .color(self.palette.text),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(
                                "Un navigateur indépendant, construit autour du moteur Archia.",
                            )
                            .size(15.0)
                            .color(self.palette.muted),
                        );
                        ui.add_space(28.0);
                        ui.horizontal(|ui| {
                            let width = ui.available_width().min(620.0);
                            let response = ui.add_sized(
                                [width, 42.0],
                                egui::TextEdit::singleline(&mut self.home_query)
                                    .hint_text("Rechercher sur le web ou saisir une adresse"),
                            );
                            if response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                let target = self.home_query.clone();
                                self.address = target.clone();
                                self.navigate(&target);
                            }
                        });
                        ui.add_space(12.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Rechercher  →")
                                        .color(self.palette.window)
                                        .strong(),
                                )
                                .fill(self.palette.accent),
                            )
                            .clicked()
                        {
                            let target = self.home_query.clone();
                            self.address = target.clone();
                            self.navigate(&target);
                        }
                    });
                }
            });
        if self.bookmark_import_open {
            egui::Window::new("Importer des favoris")
                .open(&mut self.bookmark_import_open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label("Collez des URL, un fichier TSV (titre + URL) ou des liens Markdown.");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.bookmark_import_text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .hint_text("https://example.org/"),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Importer").clicked() {
                            self.import_bookmarks();
                        }
                        if ui.button("Effacer").clicked() {
                            self.bookmark_import_text.clear();
                        }
                    });
                });
        }
    }
}

fn is_safe_clipboard_text(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= 4_096 && !value.chars().any(char::is_control)
}

fn decode_bookmark_html_text(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .trim()
        .to_owned()
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    decode_bookmark_html_text(&output)
}

fn parse_netscape_bookmarks(text: &str) -> Vec<SavedPage> {
    let lower = text.to_ascii_lowercase();
    let mut imported = Vec::new();
    let mut cursor = 0;
    while imported.len() < 500 {
        let Some(relative_start) = lower[cursor..].find("<a ") else {
            break;
        };
        let start = cursor + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let tag_end = start + relative_end;
        let tag = &text[start..=tag_end];
        let tag_lower = &lower[start..=tag_end];
        let Some(href_start) = tag_lower.find("href") else {
            cursor = tag_end + 1;
            continue;
        };
        let after_href = tag[href_start + 4..].trim_start();
        let Some(after_equals) = after_href.strip_prefix('=') else {
            cursor = tag_end + 1;
            continue;
        };
        let attribute = after_equals.trim_start();
        let raw_url = if let Some(value) = attribute.strip_prefix('"') {
            let Some(end) = value.find('"') else {
                cursor = tag_end + 1;
                continue;
            };
            &value[..end]
        } else if let Some(value) = attribute.strip_prefix('\'') {
            let Some(end) = value.find('\'') else {
                cursor = tag_end + 1;
                continue;
            };
            &value[..end]
        } else {
            let end = attribute
                .find(char::is_whitespace)
                .unwrap_or(attribute.len());
            &attribute[..end]
        };
        let Some(close_relative) = lower[tag_end + 1..].find("</a>") else {
            break;
        };
        let close_start = tag_end + 1 + close_relative;
        let title = strip_html_tags(&text[tag_end + 1..close_start]);
        if let Ok(url) = Url::parse(&decode_bookmark_html_text(raw_url)) {
            if matches!(url.scheme(), "http" | "https") {
                let title = if title.is_empty() {
                    url.host().to_owned()
                } else {
                    title
                };
                if !imported
                    .iter()
                    .any(|item: &SavedPage| item.url == url.to_string())
                {
                    imported.push(SavedPage {
                        title,
                        url: url.to_string(),
                    });
                }
            }
        }
        cursor = close_start + 4;
    }
    imported
}

fn escape_csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn escape_bookmark_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn export_bookmarks_html(bookmarks: &[SavedPage]) -> String {
    let entries = bookmarks
        .iter()
        .filter_map(|item| {
            let url = Url::parse(&item.url).ok()?;
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            Some(format!(
                "<DT><A HREF=\"{}\">{}</A>",
                escape_bookmark_html(&url.to_string()),
                escape_bookmark_html(&item.title)
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Archia Browser Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n{entries}\n</DL><p>\n"
    )
}

fn parse_bookmark_text(text: &str) -> Vec<SavedPage> {
    if text.to_ascii_lowercase().contains("<a ") {
        return parse_netscape_bookmarks(text);
    }
    let mut imported = Vec::new();
    for line in text.lines().take(500) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (title, raw_url) = if let Some(rest) = line.strip_prefix("- [") {
            if let Some((label, target)) = rest.split_once("](<") {
                (
                    label
                        .trim_end_matches(']')
                        .replace("\\[", "[")
                        .replace("\\]", "]"),
                    target.trim_end_matches(')').trim_end_matches('>'),
                )
            } else {
                (String::new(), line)
            }
        } else if let Some((first, second)) = line.split_once('\t') {
            if Url::parse(first.trim()).is_ok() {
                (second.trim().to_owned(), first.trim())
            } else {
                (first.trim().to_owned(), second.trim())
            }
        } else {
            (String::new(), line)
        };
        if let Ok(url) = Url::parse(raw_url.trim()) {
            if matches!(url.scheme(), "http" | "https") {
                let title = if title.trim().is_empty() {
                    url.host().to_owned()
                } else {
                    title
                };
                if !imported
                    .iter()
                    .any(|item: &SavedPage| item.url == url.to_string())
                {
                    imported.push(SavedPage {
                        title,
                        url: url.to_string(),
                    });
                }
            }
        }
    }
    imported
}

fn downloads_directory() -> PathBuf {
    let base = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Downloads").join("Archia Browser")
}

fn safe_download_filename(value: &str) -> String {
    let leaf = Path::new(value)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let cleaned = leaf
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(*character, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                && *character != char::from(92)
        })
        .take(180)
        .collect::<String>();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download.bin".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn unique_download_destination(directory: &Path, filename: &str) -> PathBuf {
    let candidate = directory.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_string());
    for index in 1..=10_000 {
        let name = match extension.as_deref() {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = directory.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("download-{}.bin", std::process::id()))
}

fn recent_downloads() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(downloads_directory()) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file()
                || entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".archia-pending-")
            {
                return None;
            }
            Some((
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                entry.path(),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    files.into_iter().take(12).map(|(_, path)| path).collect()
}

fn open_local_path(path: &Path) {
    #[cfg(windows)]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    let _ = result;
}

#[cfg(windows)]
fn browser_data_directory() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("ArchiaBrowser")
}

#[cfg(not(windows))]
fn browser_data_directory() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("archia-browser")
}

fn load_saved_pages(path: &std::path::Path) -> Vec<SavedPage> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let (title, url) = line.split_once('\t')?;
            if url.is_empty() {
                return None;
            }
            Some(SavedPage {
                title: decode_field(title),
                url: decode_field(url),
            })
        })
        .take(500)
        .collect()
}

fn save_saved_pages(path: &std::path::Path, pages: &[SavedPage]) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let mut contents = String::new();
    for page in pages.iter().take(500) {
        contents.push_str(&encode_query(&page.title));
        contents.push('\t');
        contents.push_str(&encode_query(&page.url));
        contents.push('\n');
    }
    let temporary = path.with_extension("tsv.tmp");
    if std::fs::write(&temporary, contents).is_ok() && std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::rename(temporary, path);
    }
}

fn decode_field(value: &str) -> String {
    let source = value.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' && index + 2 < source.len() {
            if let Ok(hex) = std::str::from_utf8(&source[index + 1..index + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    index += 3;
                    continue;
                }
            }
        }
        decoded.push(source[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn looks_like_host(input: &str) -> bool {
    let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
    authority.starts_with('[') && authority.contains(']')
        || authority == "localhost"
        || authority.starts_with("localhost:")
        || authority.parse::<std::net::IpAddr>().is_ok()
        || authority.contains('.')
}

fn encode_query(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            encoded.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

pub fn run() {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        "Archia Browser",
        options,
        Box::new(|cc| Ok(Box::new(ArchiaApp::new(cc)))),
    ) {
        eprintln!("Impossible de démarrer Archia Browser : {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_query, looks_like_host};

    #[test]
    fn address_detection_distinguishes_hosts_from_searches() {
        assert!(looks_like_host("archiaos.org"));
        assert!(looks_like_host("localhost:8080"));
        assert!(looks_like_host("192.0.2.1"));
        assert!(!looks_like_host("meilleur navigateur rust"));
    }

    #[test]
    fn search_encoding_is_utf8_percent_encoded() {
        assert_eq!(encode_query("archia browser"), "archia%20browser");
        assert_eq!(encode_query("été"), "%C3%A9t%C3%A9");
    }

    #[test]
    fn saved_page_fields_round_trip_utf8_and_delimiters() {
        let value = "Été\tArchia Browser";
        assert_eq!(super::decode_field(&encode_query(value)), value);
    }

    #[test]
    fn settings_round_trip_and_reject_unsafe_homepages() {
        let path = std::env::temp_dir().join(format!("archia-settings-{}.ini", std::process::id()));
        let mut settings = super::BrowserSettings::default();
        settings.search_provider = super::SearchProvider::DuckDuckGo;
        settings.user_agent = super::UserAgentProfile::ChromiumCompatible;
        settings.homepage = "https://example.org/".to_owned();
        settings.restore_session = false;
        settings.save(&path);
        let loaded = super::BrowserSettings::load(&path);
        assert_eq!(loaded, settings);
        std::fs::write(&path, "homepage=file:///etc/passwd\n").unwrap();
        let rejected = super::BrowserSettings::load(&path);
        assert_eq!(
            rejected.homepage,
            super::BrowserSettings::default().homepage
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bookmark_import_accepts_urls_tsv_and_markdown_but_rejects_unsafe_schemes() {
        let imported = super::parse_bookmark_text(
            "https://example.org/\nExample\thttps://example.net/\n- [Docs](<https://docs.example.org/>)\nfile:///etc/passwd"
        );
        assert_eq!(imported.len(), 3);
        assert_eq!(imported[0].url, "https://example.org/");
        assert_eq!(imported[1].title, "Example");
        assert_eq!(imported[2].title, "Docs");
    }

    #[test]
    fn netscape_bookmark_import_and_html_export_round_trip() {
        let imported = super::parse_bookmark_text(
            "<DL><DT><A HREF=\"https://example.org/\" ADD_DATE=\"1\">Example &amp; Docs</A></DL>",
        );
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].title, "Example & Docs");
        let exported = super::export_bookmarks_html(&imported);
        assert!(exported.contains("https://example.org/"));
        assert!(exported.contains("Example &amp; Docs"));
    }

    #[test]
    fn download_filename_sanitization_removes_path_and_reserved_characters() {
        assert_eq!(
            super::safe_download_filename("../../bad:name?.txt"),
            "badname.txt"
        );
        assert_eq!(super::safe_download_filename("..."), "download.bin");
    }
}
