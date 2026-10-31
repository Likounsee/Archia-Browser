//! Native Windows desktop frontend for Archia Browser.
//!
//! This first GUI milestone deliberately uses Win32 controls and the existing
//! Rust document engine. It does not embed Edge/WebView2 or another browser.

use std::{ffi::c_void, mem, ptr};

use archia_browser::{
    layout::LayoutViewport,
    net::{
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine, ResourceKind},
        DocumentLoader, HttpTransport, Request, Url,
    },
    surface::SoftwareSurface,
    Browser,
};

type Handle = *mut c_void;
type Hwnd = Handle;
type Hinstance = Handle;
type Wparam = usize;
type Lparam = isize;
type Lresult = isize;

const WM_CREATE: u32 = 0x0001;
const WM_SIZE: u32 = 0x0005;
const WM_PAINT: u32 = 0x000F;
const WM_VSCROLL: u32 = 0x0115;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_COMMAND: u32 = 0x0111;
const WM_NCCREATE: u32 = 0x0081;
const WM_DESTROY: u32 = 0x0002;
const GWLP_USERDATA: i32 = -21;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF0000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_CHILD: u32 = 0x40000000;
const WS_BORDER: u32 = 0x00800000;
const WS_VSCROLL: u32 = 0x00200000;
const WS_HSCROLL: u32 = 0x00100000;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const SW_SHOW: i32 = 5;
const SW_HIDE: i32 = 0;
const COLOR_WINDOW: usize = 5;
const SB_VERT: i32 = 1;
const SIF_RANGE: u32 = 0x0001;
const SIF_PAGE: u32 = 0x0002;
const SIF_POS: u32 = 0x0004;
const SIF_TRACKPOS: u32 = 0x0010;
const SB_LINEUP: usize = 0;
const SB_LINEDOWN: usize = 1;
const SB_PAGEUP: usize = 2;
const SB_PAGEDOWN: usize = 3;
const SB_THUMBPOSITION: usize = 4;
const SB_THUMBTRACK: usize = 5;
const SB_TOP: usize = 6;
const SB_BOTTOM: usize = 7;

const ID_BACK: usize = 1001;
const ID_FORWARD: usize = 1002;
const ID_RELOAD: usize = 1003;
const ID_GO: usize = 1004;
const ID_NEW_TAB: usize = 1005;
const ID_ADDRESS: usize = 1100;
const ID_PAGE: usize = 1200;
const ID_TAB_BASE: usize = 2000;
const MAX_GUI_TABS: usize = 100;

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    hdc: Handle,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct ScrollInfo {
    size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    pos: i32,
    track_pos: i32,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pixels_per_meter: i32,
    y_pixels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Message {
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
    time: u32,
    point: Point,
}

#[repr(C)]
struct CreateStructW {
    create_params: *mut c_void,
    instance: Hinstance,
    menu: Handle,
    parent: Hwnd,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
}

#[repr(C)]
struct WindowClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    class_extra: i32,
    window_extra: i32,
    instance: Hinstance,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WindowClassW) -> u16;
    fn LoadCursorW(instance: Hinstance, name: *const u16) -> Handle;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: Handle,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, message: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
    fn GetMessageW(message: *mut Message, hwnd: Hwnd, min: u32, max: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> Lresult;
    fn PostQuitMessage(exit_code: i32);
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn MoveWindow(hwnd: Hwnd, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
    fn SendMessageW(hwnd: Hwnd, message: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
    fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
    fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
    fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
    fn SetScrollInfo(hwnd: Hwnd, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
    fn GetScrollInfo(hwnd: Hwnd, bar: i32, info: *mut ScrollInfo) -> i32;
    fn ShowScrollBar(hwnd: Hwnd, bar: i32, show: i32) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn SetDIBitsToDevice(
        hdc: Handle,
        x_dest: i32,
        y_dest: i32,
        width: u32,
        height: u32,
        x_src: i32,
        y_src: i32,
        start_scan: u32,
        scan_lines: u32,
        bits: *const c_void,
        info: *const BitmapInfo,
        usage: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(module_name: *const u16) -> Hinstance;
}

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

struct AppState {
    browser: Browser,
    loader: Loader,
    window: Hwnd,
    back: Hwnd,
    forward: Hwnd,
    reload: Hwnd,
    address: Hwnd,
    go: Hwnd,
    new_tab: Hwnd,
    page: Hwnd,
    surface: Option<SoftwareSurface>,
    surface_dirty: bool,
    scroll_y: u32,
    dib_pixels: Vec<u8>,
    tabs: Vec<(archia_browser::core::tabs::TabId, Hwnd)>,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn create_control(
    parent: Hwnd,
    instance: Hinstance,
    class: &str,
    text: &str,
    style: u32,
    id: usize,
) -> Hwnd {
    let class = wide(class);
    let text = wide(text);
    CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        0,
        0,
        parent,
        id as Handle,
        instance,
        ptr::null_mut(),
    )
}

unsafe fn app_state(hwnd: Hwnd) -> Option<&'static mut AppState> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    pointer.as_mut()
}

fn parse_address(input: &str) -> Result<Url, archia_browser::net::UrlError> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    Url::parse(&address)
}

unsafe fn read_control_text(hwnd: Hwnd) -> String {
    let length = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut buffer = vec![0u16; length.saturating_add(1).min(32_769)];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied])
}

unsafe fn set_control_text(hwnd: Hwnd, value: &str) {
    let value = wide(value);
    let _ = SetWindowTextW(hwnd, value.as_ptr());
}

fn welcome_text() -> &'static str {
    "Bienvenue dans Archia Browser\n\nSaisissez une adresse dans la barre ci-dessus puis cliquez sur Aller.\n\nLe moteur de rendu natif affiche ici les pages HTML/CSS."
}

unsafe fn current_viewport(state: &AppState) -> LayoutViewport {
    let mut client = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if GetClientRect(state.window, &mut client) == 0 {
        return LayoutViewport::new(1200, 760);
    }
    LayoutViewport::new(
        client
            .right
            .saturating_sub(client.left)
            .saturating_sub(20)
            .max(1) as u32,
        client
            .bottom
            .saturating_sub(client.top)
            .saturating_sub(102)
            .max(1) as u32,
    )
}

unsafe fn update_scrollbar(state: &mut AppState) {
    let viewport = current_viewport(state);
    let content_height = state
        .browser
        .current_page()
        .map(|page| page.layout.rect.height)
        .unwrap_or(viewport.height);
    let max_scroll = content_height.saturating_sub(viewport.height);
    state.scroll_y = state.scroll_y.min(max_scroll);
    let info = ScrollInfo {
        size: mem::size_of::<ScrollInfo>() as u32,
        mask: SIF_RANGE | SIF_PAGE | SIF_POS,
        min: 0,
        max: content_height.saturating_sub(1).min(i32::MAX as u32) as i32,
        page: viewport.height,
        pos: state.scroll_y.min(i32::MAX as u32) as i32,
        track_pos: 0,
    };
    let _ = SetScrollInfo(state.window, SB_VERT, &info, 1);
    let _ = ShowScrollBar(state.window, SB_VERT, if max_scroll > 0 { 1 } else { 0 });
}

unsafe fn scroll_to(state: &mut AppState, requested: i64) {
    let viewport = current_viewport(state);
    let content_height = state
        .browser
        .current_page()
        .map(|page| page.layout.rect.height)
        .unwrap_or(viewport.height);
    let max_scroll = content_height.saturating_sub(viewport.height);
    let next = requested.clamp(0, max_scroll as i64) as u32;
    if next != state.scroll_y {
        state.scroll_y = next;
        state.surface_dirty = true;
        update_scrollbar(state);
        let _ = InvalidateRect(state.window, ptr::null(), 1);
    }
}

unsafe fn refresh_page(state: &mut AppState) {
    state.surface_dirty = true;
    state.scroll_y = 0;
    if state.browser.current_page().is_some() {
        set_control_text(state.page, "");
        let _ = ShowWindow(state.page, SW_HIDE);
    } else {
        set_control_text(state.page, welcome_text());
        let _ = ShowWindow(state.page, SW_SHOW);
    }
    let title = state
        .browser
        .current_page()
        .and_then(|page| page.title())
        .unwrap_or_else(|| "Nouvel onglet".to_owned());
    let address = state
        .browser
        .current_url()
        .map(ToString::to_string)
        .unwrap_or_default();
    set_control_text(state.address, &address);
    let caption = format!("{title} — Archia Browser");
    let caption = wide(&caption);
    let _ = SetWindowTextW(state.window, caption.as_ptr());
    let _ = InvalidateRect(state.window, ptr::null(), 1);
    update_scrollbar(state);
    if let Some(active_id) = state.browser.active_tab_id() {
        if let Some((index, (_, handle))) = state
            .tabs
            .iter()
            .enumerate()
            .find(|(_, (id, _))| *id == active_id)
        {
            let short_title = title.chars().take(18).collect::<String>();
            let caption = format!("{} · {}", index + 1, short_title);
            set_control_text(*handle, &caption);
        }
    }
}

unsafe fn navigate(state: &mut AppState, address: &str) {
    let url = match parse_address(address) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        Ok(_) => {
            set_control_text(
                state.page,
                "Archia Browser n'autorise pour le moment que HTTP et HTTPS.",
            );
            return;
        }
        Err(error) => {
            set_control_text(state.page, &format!("Adresse invalide : {error:?}"));
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
        format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ArchiaBrowser/{}",
            state.browser.version()
        ),
    );
    request.policy.resource_kind = ResourceKind::Document;
    request.policy.top_level_navigation = true;

    set_control_text(state.page, "Chargement de la page…");
    let _ = ShowWindow(state.page, SW_SHOW);
    let _ = SendMessageW(state.page, EM_SETLIMITTEXT, 2 * 1024 * 1024, 0);
    let viewport = current_viewport(state);
    match state
        .browser
        .load_request(&state.loader, &request, viewport)
    {
        Ok(_) => refresh_page(state),
        Err(error) => {
            let message = format!(
                "Impossible de charger cette page.\n\n{error:?}\n\nVérifiez l'adresse et votre connexion Internet."
            );
            set_control_text(state.page, &message);
            let _ = ShowWindow(state.page, SW_SHOW);
        }
    }
}

unsafe fn layout_controls(state: &AppState, width: i32, height: i32) {
    let margin = 10;
    let button_y = 48;
    let button_h = 30;
    let tab_area_width = (width - 2 * margin - 70).max(100);
    let tab_width = (tab_area_width / state.tabs.len().max(1) as i32).clamp(90, 180);
    for (index, (_, handle)) in state.tabs.iter().enumerate() {
        let _ = MoveWindow(
            *handle,
            margin + index as i32 * tab_width,
            8,
            tab_width - 4,
            30,
            1,
        );
    }
    let _ = MoveWindow(state.new_tab, width - 62, 8, 52, 30, 1);
    let _ = MoveWindow(state.back, margin, button_y, 38, button_h, 1);
    let _ = MoveWindow(state.forward, margin + 42, button_y, 38, button_h, 1);
    let _ = MoveWindow(state.reload, margin + 84, button_y, 38, button_h, 1);
    let _ = MoveWindow(
        state.address,
        margin + 128,
        button_y,
        (width - 260).max(100),
        button_h,
        1,
    );
    let _ = MoveWindow(state.go, width - 118, button_y, 108, button_h, 1);
    let _ = MoveWindow(
        state.page,
        margin,
        button_y + button_h + 12,
        (width - 2 * margin).max(100),
        (height - button_y - button_h - 32).max(100),
        1,
    );
}

unsafe extern "system" fn window_proc(
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
) -> Lresult {
    if message == WM_NCCREATE {
        let create = &*(l_param as *const CreateStructW);
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.create_params as isize);
    }

    match message {
        WM_CREATE => {
            let Some(state) = app_state(hwnd) else {
                return -1;
            };
            state.window = hwnd;
            let instance = GetModuleHandleW(ptr::null());
            state.back = create_control(hwnd, instance, "BUTTON", "←", 0, ID_BACK);
            state.forward = create_control(hwnd, instance, "BUTTON", "→", 0, ID_FORWARD);
            state.reload = create_control(hwnd, instance, "BUTTON", "↻", 0, ID_RELOAD);
            state.address = create_control(
                hwnd,
                instance,
                "EDIT",
                "https://example.org",
                WS_BORDER | ES_AUTOHSCROLL,
                ID_ADDRESS,
            );
            state.go = create_control(hwnd, instance, "BUTTON", "Aller", 0, ID_GO);
            state.new_tab = create_control(hwnd, instance, "BUTTON", "+", 0, ID_NEW_TAB);
            let initial_tab = state
                .browser
                .active_tab_id()
                .expect("browser starts with one tab");
            let initial_button = create_control(
                hwnd,
                instance,
                "BUTTON",
                "1 · Nouvel onglet",
                0,
                ID_TAB_BASE,
            );
            state.tabs.push((initial_tab, initial_button));
            state.page = create_control(
                hwnd,
                instance,
                "EDIT",
                "",
                WS_BORDER | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
                ID_PAGE,
            );
            let _ = SendMessageW(state.page, EM_SETLIMITTEXT, 2 * 1024 * 1024, 0);
            let _ = SendMessageW(state.address, 0x0030, 1, 0); // WM_SETFONT
            let _ = SendMessageW(state.page, 0x0030, 1, 0);
            refresh_page(state);
            0
        }
        WM_PAINT => {
            let mut paint: PaintStruct = mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut paint);
            if !hdc.is_null() {
                if let Some(state) = app_state(hwnd) {
                    let mut client = Rect {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if GetClientRect(hwnd, &mut client) != 0
                        && state.browser.current_page().is_some()
                    {
                        let width = client
                            .right
                            .saturating_sub(client.left)
                            .saturating_sub(20)
                            .max(1) as u32;
                        let height = client
                            .bottom
                            .saturating_sub(client.top)
                            .saturating_sub(102)
                            .max(1) as u32;
                        let needs_surface = state.surface.as_ref().is_none_or(|surface| {
                            surface.width() != width || surface.height() != height
                        });
                        if needs_surface {
                            state.surface = SoftwareSurface::try_new(width, height);
                            state.dib_pixels.clear();
                            state.surface_dirty = true;
                        }
                        if state.surface_dirty {
                            if let Some(surface) = state.surface.as_mut() {
                                if state.browser.render_current_page_at(surface, state.scroll_y) {
                                    let required = (width as usize)
                                        .saturating_mul(height as usize)
                                        .saturating_mul(4);
                                    if state.dib_pixels.len() != required {
                                        state.dib_pixels.resize(required, 0);
                                    }
                                    let source_pixels = surface.pixels().as_chunks::<4>().0;
                                    let target_pixels = state.dib_pixels.as_chunks_mut::<4>().0;
                                    for (source, target) in
                                        source_pixels.iter().zip(target_pixels.iter_mut())
                                    {
                                        *target = [source[2], source[1], source[0], 0];
                                    }
                                    state.surface_dirty = false;
                                }
                            }
                        }
                        if !state.dib_pixels.is_empty() {
                            let required = state.dib_pixels.len() as u32;
                            let info = BitmapInfo {
                                header: BitmapInfoHeader {
                                    size: mem::size_of::<BitmapInfoHeader>() as u32,
                                    width: width as i32,
                                    height: -(height as i32),
                                    planes: 1,
                                    bit_count: 32,
                                    compression: 0,
                                    size_image: required,
                                    x_pixels_per_meter: 0,
                                    y_pixels_per_meter: 0,
                                    colors_used: 0,
                                    colors_important: 0,
                                },
                                colors: [0],
                            };
                            let _ = SetDIBitsToDevice(
                                hdc,
                                10,
                                90,
                                width,
                                height,
                                0,
                                0,
                                0,
                                height,
                                state.dib_pixels.as_ptr().cast(),
                                &info,
                                0,
                            );
                        }
                    }
                }
            }
            let _ = EndPaint(hwnd, &paint);
            0
        }
        WM_SIZE => {
            if let Some(state) = app_state(hwnd) {
                let width = (l_param as u32 & 0xffff) as i32;
                let height = ((l_param as u32 >> 16) & 0xffff) as i32;
                layout_controls(state, width, height);
                let viewport = LayoutViewport::new(
                    width.saturating_sub(20).max(1) as u32,
                    height.saturating_sub(102).max(1) as u32,
                );
                if state.browser.relayout_current_page(viewport) {
                    state.surface_dirty = true;
                    update_scrollbar(state);
                    let _ = InvalidateRect(hwnd, ptr::null(), 1);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = app_state(hwnd) {
                let delta = ((w_param >> 16) as u16) as i16 as i64;
                let lines = (delta / 120).saturating_mul(3);
                let distance = lines.saturating_mul(48);
                scroll_to(state, state.scroll_y as i64 - distance);
            }
            0
        }
        WM_VSCROLL => {
            if let Some(state) = app_state(hwnd) {
                let command = w_param & 0xffff;
                let current = state.scroll_y as i64;
                let viewport = current_viewport(state);
                let next = match command {
                    SB_LINEUP => current - 48,
                    SB_LINEDOWN => current + 48,
                    SB_PAGEUP => current - viewport.height as i64,
                    SB_PAGEDOWN => current + viewport.height as i64,
                    SB_TOP => 0,
                    SB_BOTTOM => i64::MAX,
                    SB_THUMBPOSITION | SB_THUMBTRACK => {
                        let mut info = ScrollInfo {
                            size: mem::size_of::<ScrollInfo>() as u32,
                            mask: SIF_TRACKPOS,
                            min: 0,
                            max: 0,
                            page: 0,
                            pos: 0,
                            track_pos: 0,
                        };
                        if GetScrollInfo(hwnd, SB_VERT, &mut info) != 0 {
                            info.track_pos as i64
                        } else {
                            current
                        }
                    }
                    _ => current,
                };
                scroll_to(state, next);
            }
            0
        }
        WM_COMMAND => {
            let id = w_param & 0xffff;
            if let Some(state) = app_state(hwnd) {
                match id {
                    ID_GO => {
                        let address = read_control_text(state.address);
                        navigate(state, &address);
                    }
                    ID_BACK => {
                        if state
                            .browser
                            .go_back(&state.loader, current_viewport(state))
                            .is_ok()
                        {
                            refresh_page(state);
                        }
                    }
                    ID_FORWARD => {
                        if state
                            .browser
                            .go_forward(&state.loader, current_viewport(state))
                            .is_ok()
                        {
                            refresh_page(state);
                        }
                    }
                    ID_RELOAD => {
                        if state
                            .browser
                            .reload(&state.loader, current_viewport(state))
                            .is_ok()
                        {
                            refresh_page(state);
                        }
                    }
                    ID_NEW_TAB => {
                        if state.tabs.len() < MAX_GUI_TABS {
                            let tab_id = state.browser.new_tab();
                            let index = state.tabs.len();
                            let caption = format!("{} · Nouvel onglet", index + 1);
                            let button = create_control(
                                hwnd,
                                GetModuleHandleW(ptr::null()),
                                "BUTTON",
                                &caption,
                                0,
                                ID_TAB_BASE + index,
                            );
                            state.tabs.push((tab_id, button));
                            set_control_text(state.address, "");
                            refresh_page(state);
                            let mut client = Rect {
                                left: 0,
                                top: 0,
                                right: 1180,
                                bottom: 820,
                            };
                            if GetClientRect(hwnd, &mut client) != 0 {
                                layout_controls(
                                    state,
                                    client.right.saturating_sub(client.left),
                                    client.bottom.saturating_sub(client.top),
                                );
                            }
                        }
                    }
                    tab_id if (ID_TAB_BASE..ID_TAB_BASE + MAX_GUI_TABS).contains(&tab_id) => {
                        let index = tab_id - ID_TAB_BASE;
                        if let Some((id, _)) = state.tabs.get(index) {
                            state.browser.select_tab(*id);
                            refresh_page(state);
                        }
                    }
                    _ => {}
                }
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, w_param, l_param),
    }
}

pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class_name = wide("ArchiaBrowserWindow");
        let window_title = wide("Archia Browser");
        let class = WindowClassW {
            style: 0x0002 | 0x0001,
            wnd_proc: Some(window_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: ptr::null_mut(),
            cursor: LoadCursorW(ptr::null_mut(), 32512usize as *const u16),
            background: (COLOR_WINDOW + 1) as Handle,
            menu_name: ptr::null(),
            class_name: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            eprintln!("Impossible d'enregistrer la fenêtre Archia Browser.");
            return;
        }

        let pipeline = NetworkPipeline::new(BrowserPolicy);
        let browser = Browser::new();
        let loader = browser.document_loader(pipeline, HttpTransport::new());
        let mut state = Box::new(AppState {
            browser,
            loader,
            window: ptr::null_mut(),
            back: ptr::null_mut(),
            forward: ptr::null_mut(),
            reload: ptr::null_mut(),
            address: ptr::null_mut(),
            go: ptr::null_mut(),
            new_tab: ptr::null_mut(),
            page: ptr::null_mut(),
            surface: None,
            surface_dirty: true,
            scroll_y: 0,
            dib_pixels: Vec::new(),
            tabs: Vec::new(),
        });
        let state_ptr = &mut *state as *mut AppState;
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_VSCROLL,
            100,
            100,
            1180,
            820,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            state_ptr.cast(),
        );
        if hwnd.is_null() {
            eprintln!("Impossible de créer la fenêtre Archia Browser.");
            return;
        }

        let mut message: Message = mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
}
