//! Shared cross-platform desktop interface for Archia Browser.
//!
//! The window, widgets and browser chrome are rendered by egui on wgpu.
//! Platform-specific windowing and GPU backends are provided by eframe/winit;
//! this module contains no Win32, GDI or Direct2D calls.

use std::time::Duration;

use archia_browser::{
    layout::LayoutViewport,
    net::{
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine, ResourceKind},
        DocumentLoader, HttpTransport, Request, Url,
    },
    surface::SoftwareSurface,
    Browser,
};
use eframe::egui;

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

const GRAPHITE: egui::Color32 = egui::Color32::from_rgb(19, 22, 27);
const PANEL: egui::Color32 = egui::Color32::from_rgb(25, 29, 35);
const PANEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(34, 39, 47);
const LIME: egui::Color32 = egui::Color32::from_rgb(190, 242, 100);
const TEXT: egui::Color32 = egui::Color32::from_rgb(238, 241, 235);
const MUTED: egui::Color32 = egui::Color32::from_rgb(147, 155, 163);

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
        style.visuals.selection.stroke = egui::Stroke::new(1.0, LIME);
        style.visuals.widgets.inactive.bg_fill = PANEL_LIGHT;
        style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 56, 63);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(57, 75, 44);
        style.visuals.widgets.open.bg_fill = PANEL_LIGHT;
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(11.0, 7.0);
        cc.egui_ctx.set_style(style);

        let mut browser = Browser::new();
        let first_tab = browser.active_tab_id().expect("Browser creates an initial tab");
        let loader = browser.document_loader(
            NetworkPipeline::new(BrowserPolicy),
            HttpTransport::new(),
        );

        Self {
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
        }
    }

    fn navigate(&mut self, input: &str) {
        let input = input.trim();
        if input.is_empty() {
            return;
        }

        let address = if input.contains("://") {
            input.to_owned()
        } else if input.chars().any(char::is_whitespace) || !looks_like_host(input) {
            format!("https://www.google.com/search?q={}", encode_query(input))
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
        request.headers.insert(
            "accept-language".into(),
            "fr-FR,fr;q=0.9,en;q=0.8".into(),
        );
        request.headers.insert(
            "user-agent".into(),
            format!(
                "Mozilla/5.0 (compatible; ArchiaBrowser/{})",
                self.browser.version()
            ),
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
    }

    fn close_tab(&mut self, id: archia_browser::core::tabs::TabId) {
        if self.tabs.len() <= 1 {
            self.status = "Le dernier onglet ne peut pas être fermé.".to_owned();
            return;
        }
        if self.browser.close_tab(id) {
            self.tabs.retain(|tab| *tab != id);
            self.address = self
                .browser
                .current_url()
                .map(ToString::to_string)
                .unwrap_or_default();
            self.page_texture = None;
            self.page_dirty = true;
            self.status = "Onglet fermé".to_owned();
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
                self.page_texture = Some(ctx.load_texture(
                    "archia-page",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
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
}

impl eframe::App for ArchiaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("archia_tabs")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for id in self.tabs.clone() {
                        let active = self.browser.active_tab_id() == Some(id);
                        let title = self.tab_title(id);
                        ui.horizontal(|ui| {
                            let response = ui.selectable_label(active, title);
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
            .frame(egui::Frame::new().fill(GRAPHITE).inner_margin(egui::Margin::symmetric(14, 10)))
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
                    if address.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        let target = self.address.clone();
                        self.navigate(&target);
                    }
                    if ui.button("Aller").clicked() {
                        let target = self.address.clone();
                        self.navigate(&target);
                    }
                });
            });

        egui::TopBottomPanel::bottom("archia_status")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 6)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("●").color(LIME));
                    ui.label(egui::RichText::new(&self.status).color(MUTED).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("ARCHIA BROWSER · {}", self.browser.version()))
                                .color(MUTED)
                                .small(),
                        );
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(GRAPHITE).inner_margin(0))
            .show(ctx, |ui| {
                let width = ui.available_width().max(480.0) as u32;
                if width != self.last_viewport_width {
                    self.last_viewport_width = width;
                    if self.browser.current_page().is_some() {
                        self.browser.relayout_current_page(LayoutViewport::new(width, 760));
                        self.page_dirty = true;
                    }
                }

                if self.browser.current_page().is_some() {
                    self.update_page_texture(ctx, width);
                    if let Some(texture) = self.page_texture.as_ref() {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Image::new((texture.id(), self.page_texture_size))
                                        .sense(egui::Sense::click_and_drag()),
                                );
                            });
                    }
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.20).max(48.0));
                        ui.label(
                            egui::RichText::new("ARCHIA")
                                .size(18.0)
                                .strong()
                                .color(LIME)
                                .letter_spacing(3.0),
                        );
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("Le web, à votre façon.")
                                .size(34.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Un navigateur indépendant, construit autour du moteur Archia.")
                                .size(15.0)
                                .color(MUTED),
                        );
                        ui.add_space(28.0);
                        ui.horizontal(|ui| {
                            let width = ui.available_width().min(620.0);
                            let response = ui.add_sized(
                                [width, 42.0],
                                egui::TextEdit::singleline(&mut self.home_query)
                                    .hint_text("Rechercher sur le web ou saisir une adresse"),
                            );
                            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                                let target = self.home_query.clone();
                                self.address = target.clone();
                                self.navigate(&target);
                            }
                        });
                        ui.add_space(12.0);
                        if ui.button(egui::RichText::new("Rechercher  →").color(GRAPHITE).strong())
                            .fill(LIME)
                            .clicked()
                        {
                            let target = self.home_query.clone();
                            self.address = target.clone();
                            self.navigate(&target);
                        }
                    });
                }
            });
    }
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
}
