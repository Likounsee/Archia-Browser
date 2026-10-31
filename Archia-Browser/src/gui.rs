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
    theme::{ThemeColors, ThemeManager},
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
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_COMMAND: u32 = 0x0111;
const WM_NCCREATE: u32 = 0x0081;
const WM_DESTROY: u32 = 0x0002;
const WM_KEYDOWN: u32 = 0x0100;
const VK_RETURN: usize = 0x0D;
const VK_L: usize = 0x4C;
const VK_T: usize = 0x54;
const VK_W: usize = 0x57;
const VK_R: usize = 0x52;
const VK_F5: usize = 0x74;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_LEFT: usize = 0x25;
const VK_RIGHT: usize = 0x27;
const MF_STRING: u32 = 0x0000;
const MF_GRAYED: u32 = 0x0001;
const MF_SEPARATOR: u32 = 0x0800;
const TPM_RETURNCMD: u32 = 0x0100;
const ID_MENU_RELOAD: u32 = 3001;
const ID_MENU_NEW_TAB: u32 = 3002;
const ID_MENU_CLOSE_TAB: u32 = 3003;
const ID_MENU_QUIT: u32 = 3004;
const ID_MENU_THEME_ARCHIA: u32 = 3010;
const ID_MENU_THEME_CHROME: u32 = 3011;
const ID_MENU_THEME_FIREFOX: u32 = 3012;
const ID_MENU_THEME_EDGE: u32 = 3013;
const ID_MENU_HISTORY_BASE: u32 = 3200;
const MAX_MENU_HISTORY_ITEMS: usize = 8;
const GWLP_WNDPROC: i32 = -4;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const DT_SINGLELINE: u32 = 0x0020;
const DT_CENTER: u32 = 0x0001;
const DT_VCENTER: u32 = 0x0004;
const DT_END_ELLIPSIS: u32 = 0x8000;
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
const ID_HOME_SEARCH: usize = 1201;
const ID_HOME_GO: usize = 1202;
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
    fn GetParent(hwnd: Hwnd) -> Hwnd;
    fn GetKeyState(key: i32) -> i16;
    fn SetFocus(hwnd: Hwnd) -> Hwnd;
    fn CreatePopupMenu() -> Handle;
    fn AppendMenuW(menu: Handle, flags: u32, item_id: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(
        menu: Handle,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        owner: Hwnd,
        rect: *const Rect,
    ) -> u32;
    fn DestroyMenu(menu: Handle) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn CallWindowProcW(
        previous: isize,
        hwnd: Hwnd,
        message: u32,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn MoveWindow(hwnd: Hwnd, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn SendMessageW(hwnd: Hwnd, message: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
    fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
    fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
    fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
    fn SetScrollInfo(hwnd: Hwnd, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
    fn GetScrollInfo(hwnd: Hwnd, bar: i32, info: *mut ScrollInfo) -> i32;
    fn ShowScrollBar(hwnd: Hwnd, bar: i32, show: i32) -> i32;
    fn DrawTextW(hdc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn FillRect(hdc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn CreateSolidBrush(color: u32) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        char_set: u32,
        output_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> Handle;
    fn SelectObject(hdc: Handle, object: Handle) -> Handle;
    fn SetTextColor(hdc: Handle, color: u32) -> u32;
    fn SetBkColor(hdc: Handle, color: u32) -> u32;
    fn SetBkMode(hdc: Handle, mode: i32) -> i32;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bookmark {
    title: String,
    url: String,
}

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
    home_search: Hwnd,
    home_go: Hwnd,
    address_brush: Handle,
    address_proc: isize,
    home_search_proc: isize,
    surface: Option<SoftwareSurface>,
    surface_dirty: bool,
    scroll_y: u32,
    dib_pixels: Vec<u8>,
    tabs: Vec<(archia_browser::core::tabs::TabId, Hwnd)>,
    themes: Option<ThemeManager>,
    bookmarks: Vec<Bookmark>,
    bookmarks_path: std::path::PathBuf,
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

fn bookmark_storage_path() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(|base| std::path::PathBuf::from(base).join("ArchiaBrowser"))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|base| base.join(".archia-browser"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("bookmarks.tsv")
}

fn load_bookmarks(path: &std::path::Path) -> Vec<Bookmark> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let (url, title) = line.split_once('\t')?;
            let parsed = Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            Some(Bookmark {
                title: title.trim().to_owned(),
                url: parsed.to_string(),
            })
        })
        .collect()
}

fn save_bookmarks(state: &AppState) {
    if let Some(parent) = state.bookmarks_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let contents = state
        .bookmarks
        .iter()
        .map(|bookmark| {
            format!(
                "{}\t{}",
                bookmark
                    .url
                    .replace('\t', " ")
                    .replace('\r', " ")
                    .replace('\n', " "),
                bookmark
                    .title
                    .replace('\t', " ")
                    .replace('\r', " ")
                    .replace('\n', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&state.bookmarks_path, contents);
}

unsafe fn toggle_current_bookmark(state: &mut AppState) {
    let Some(url) = state.browser.current_url().map(ToString::to_string) else {
        return;
    };
    if let Some(index) = state
        .bookmarks
        .iter()
        .position(|bookmark| bookmark.url == url)
    {
        state.bookmarks.remove(index);
    } else {
        let title = state
            .browser
            .current_page()
            .and_then(|page| page.title())
            .unwrap_or_else(|| url.clone());
        state.bookmarks.push(Bookmark { title, url });
    }
    save_bookmarks(state);
    let _ = InvalidateRect(state.window, ptr::null(), 1);
}

unsafe fn app_state(hwnd: Hwnd) -> Option<&'static mut AppState> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    pointer.as_mut()
}

fn encode_search_query(input: &str) -> String {
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

fn parse_address(input: &str) -> Result<Url, archia_browser::net::UrlError> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else if input.chars().any(char::is_whitespace)
        || (!input.contains('.') && !input.starts_with("localhost") && !input.starts_with("127."))
    {
        format!(
            "https://www.google.com/search?q={}",
            encode_search_query(input)
        )
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

unsafe extern "system" fn address_edit_proc(
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
) -> Lresult {
    if message == WM_KEYDOWN {
        let parent = GetParent(hwnd);
        if let Some(state) = app_state(parent) {
            if handle_keyboard_shortcut(state, parent, w_param) {
                return 0;
            }
        }
    }
    if message == WM_KEYDOWN && w_param == VK_RETURN {
        let parent = GetParent(hwnd);
        if let Some(state) = app_state(parent) {
            let address = read_control_text(hwnd);
            navigate(state, &address);
        }
        return 0;
    }

    let parent = GetParent(hwnd);
    if let Some(state) = app_state(parent) {
        let previous_proc = if hwnd == state.address {
            state.address_proc
        } else {
            state.home_search_proc
        };
        if previous_proc != 0 {
            return CallWindowProcW(previous_proc, hwnd, message, w_param, l_param);
        }
    }
    DefWindowProcW(hwnd, message, w_param, l_param)
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
            .saturating_sub(36)
            .max(1) as u32,
        client
            .bottom
            .saturating_sub(client.top)
            .saturating_sub(132)
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
    set_control_text(state.page, "");
    let _ = ShowWindow(state.page, SW_HIDE);
    let has_page = state.browser.current_page().is_some();
    let _ = ShowWindow(state.home_search, if has_page { SW_HIDE } else { SW_SHOW });
    let _ = ShowWindow(state.home_go, if has_page { SW_HIDE } else { SW_SHOW });
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
    let _ = ShowWindow(state.home_search, SW_HIDE);
    let _ = ShowWindow(state.home_go, SW_HIDE);
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

unsafe fn activate_link_at(state: &mut AppState, hwnd: Hwnd, client_x: i32, client_y: i32) {
    let viewport = current_viewport(state);
    let content_x = client_x.saturating_sub(10);
    let content_y = client_y.saturating_sub(120);
    if content_x < 0
        || content_y < 0
        || content_x as u32 >= viewport.width
        || content_y as u32 >= viewport.height
    {
        return;
    }

    let document_y = content_y.saturating_add(state.scroll_y.min(i32::MAX as u32) as i32);
    let destination = state.browser.current_page().and_then(|page| {
        let link = page.link_at(content_x, document_y)?;
        let href = link.link_href()?.trim();
        let url = page.resolve_reference(href).ok()?;
        if url.scheme() == "file" && !page.url().is_some_and(|origin| origin.scheme() == "file") {
            return None;
        }
        let opens_new_tab = link
            .attribute("target")
            .is_some_and(|target| target.trim().eq_ignore_ascii_case("_blank"));
        Some((url.to_string(), opens_new_tab))
    });

    if let Some((url, opens_new_tab)) = destination {
        if opens_new_tab && state.tabs.len() < MAX_GUI_TABS {
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
            let mut client = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClientRect(hwnd, &mut client) != 0 {
                layout_controls(
                    state,
                    client.right.saturating_sub(client.left),
                    client.bottom.saturating_sub(client.top),
                );
            }
        }
        navigate(state, &url);
    }
}

unsafe fn paint_fill(hdc: Handle, left: i32, top: i32, right: i32, bottom: i32, color: u32) {
    let rect = Rect {
        left,
        top,
        right,
        bottom,
    };
    let brush = CreateSolidBrush(color);
    if !brush.is_null() {
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_label(
    hdc: Handle,
    label: &str,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    color: u32,
    align: u32,
) {
    let text = wide(label);
    let face = wide("Segoe UI");
    let mut rect = Rect {
        left,
        top,
        right,
        bottom,
    };
    let label_height = bottom.saturating_sub(top);
    let font_height = if label_height >= 56 {
        28
    } else if label_height >= 40 {
        20
    } else {
        14
    };
    let weight = if label_height >= 56 { 600 } else { 400 };
    let font = CreateFontW(
        -font_height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        face.as_ptr(),
    );
    let previous_font = if font.is_null() {
        ptr::null_mut()
    } else {
        SelectObject(hdc, font)
    };
    let _ = SetTextColor(hdc, color);
    let _ = SetBkMode(hdc, 1);
    let _ = DrawTextW(
        hdc,
        text.as_ptr(),
        text.len().saturating_sub(1) as i32,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | align,
    );
    if !font.is_null() {
        let _ = SelectObject(hdc, previous_font);
        let _ = DeleteObject(font);
    }
}

unsafe fn create_new_tab(state: &mut AppState, hwnd: Hwnd) {
    if state.tabs.len() >= MAX_GUI_TABS {
        return;
    }
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
    let _ = ShowWindow(button, SW_HIDE);
    state.tabs.push((tab_id, button));
    set_control_text(state.address, "");
    refresh_page(state);
}

unsafe fn close_active_tab(state: &mut AppState) {
    if state.tabs.len() <= 1 {
        return;
    }
    let Some(index) = state
        .browser
        .active_tab_id()
        .and_then(|id| state.tabs.iter().position(|(tab_id, _)| *tab_id == id))
    else {
        return;
    };
    let (id, handle) = state.tabs[index];
    if state.browser.close_tab(id) {
        let _ = DestroyWindow(handle);
        state.tabs.remove(index);
        refresh_page(state);
    }
}

unsafe fn handle_keyboard_shortcut(state: &mut AppState, hwnd: Hwnd, key: Wparam) -> bool {
    let control = GetKeyState(VK_CONTROL) < 0;
    let alt = GetKeyState(VK_MENU) < 0;
    if control {
        match key {
            VK_L => {
                let _ = SetFocus(state.address);
                let _ = SendMessageW(state.address, 0x00B1, 0, -1); // EM_SETSEL
                return true;
            }
            VK_T => {
                create_new_tab(state, hwnd);
                let _ = SetFocus(state.address);
                return true;
            }
            VK_W => {
                close_active_tab(state);
                return true;
            }
            VK_R => {
                if state
                    .browser
                    .reload(&state.loader, current_viewport(state))
                    .is_ok()
                {
                    refresh_page(state);
                }
                return true;
            }
            _ => {}
        }
    }
    if key == VK_F5 {
        if state
            .browser
            .reload(&state.loader, current_viewport(state))
            .is_ok()
        {
            refresh_page(state);
        }
        return true;
    }
    if alt && key == VK_LEFT {
        if state
            .browser
            .go_back(&state.loader, current_viewport(state))
            .is_ok()
        {
            refresh_page(state);
        }
        return true;
    }
    if alt && key == VK_RIGHT {
        if state
            .browser
            .go_forward(&state.loader, current_viewport(state))
            .is_ok()
        {
            refresh_page(state);
        }
        return true;
    }
    false
}

unsafe fn show_browser_menu(state: &mut AppState, hwnd: Hwnd) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_MENU_RELOAD, "Recharger"),
        (ID_MENU_NEW_TAB, "Nouvel onglet"),
        (ID_MENU_CLOSE_TAB, "Fermer l'onglet"),
        (ID_MENU_THEME_ARCHIA, "Theme : Archia"),
        (ID_MENU_THEME_CHROME, "Theme : Chrome"),
        (ID_MENU_THEME_FIREFOX, "Theme : Firefox"),
        (ID_MENU_THEME_EDGE, "Theme : Edge"),
        (ID_MENU_QUIT, "Quitter Archia Browser"),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    let history_heading = wide("Pages récemment visitées");
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, history_heading.as_ptr());
    for (index, entry) in state
        .browser
        .history()
        .entries()
        .rev()
        .skip(1)
        .take(MAX_MENU_HISTORY_ITEMS)
        .enumerate()
    {
        let title = entry.title().unwrap_or("Page sans titre");
        let label = wide(&format!(
            "{} — {}",
            index + 1,
            title.chars().take(42).collect::<String>()
        ));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            (ID_MENU_HISTORY_BASE as usize) + index,
            label.as_ptr(),
        );
    }
    let mut cursor = Point { x: 0, y: 0 };
    let _ = GetCursorPos(&mut cursor);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD,
        cursor.x,
        cursor.y,
        0,
        hwnd,
        ptr::null(),
    );
    let _ = DestroyMenu(menu);

    match selected {
        ID_MENU_RELOAD => {
            if state
                .browser
                .reload(&state.loader, current_viewport(state))
                .is_ok()
            {
                refresh_page(state);
            }
        }
        ID_MENU_NEW_TAB => create_new_tab(state, hwnd),
        ID_MENU_CLOSE_TAB => close_active_tab(state),
        ID_MENU_THEME_ARCHIA
        | ID_MENU_THEME_CHROME
        | ID_MENU_THEME_FIREFOX
        | ID_MENU_THEME_EDGE => {
            let theme_id = match selected {
                ID_MENU_THEME_ARCHIA => "archia",
                ID_MENU_THEME_CHROME => "chrome",
                ID_MENU_THEME_FIREFOX => "firefox",
                ID_MENU_THEME_EDGE => "edge",
                _ => unreachable!(),
            };
            if let Some(themes) = state.themes.as_mut() {
                if themes.select(theme_id).is_ok() {
                    if !state.address_brush.is_null() {
                        let _ = DeleteObject(state.address_brush);
                    }
                    state.address_brush = CreateSolidBrush(chrome_color(
                        state,
                        |colors| &colors.address_background,
                        0x00F5F3F1,
                    ));
                    let _ = InvalidateRect(hwnd, ptr::null(), 1);
                }
            }
        }
        history_id
            if (ID_MENU_HISTORY_BASE..ID_MENU_HISTORY_BASE + MAX_MENU_HISTORY_ITEMS as u32)
                .contains(&history_id) =>
        {
            let index = (history_id - ID_MENU_HISTORY_BASE) as usize;
            if let Some(url) = state
                .browser
                .history()
                .entries()
                .rev()
                .skip(1)
                .nth(index)
                .map(|entry| entry.url().to_string())
            {
                navigate(state, &url);
            }
        }
        ID_MENU_QUIT => {
            let _ = DestroyWindow(hwnd);
        }
        _ => {}
    }
}

fn parse_theme_color(value: &str) -> Option<u32> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let red = u32::from_str_radix(&value[0..2], 16).ok()?;
    let green = u32::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u32::from_str_radix(&value[4..6], 16).ok()?;
    Some((blue << 16) | (green << 8) | red)
}

fn chrome_color(state: &AppState, select: fn(&ThemeColors) -> &str, fallback: u32) -> u32 {
    state
        .themes
        .as_ref()
        .and_then(|manager| parse_theme_color(select(&manager.active_theme().manifest.colors)))
        .unwrap_or(fallback)
}

unsafe fn paint_browser_chrome(state: &AppState, hdc: Handle, width: i32) {
    // A deliberately restrained dark browser frame: separate tab strip,
    // navigation row, active-tab accent, and a quiet address-field surround.
    let toolbar = chrome_color(state, |colors| &colors.toolbar_background, 0x00211D1B);
    let window = chrome_color(state, |colors| &colors.window_background, 0x00181513);
    let active_tab = chrome_color(state, |colors| &colors.tab_active_background, 0x00443B36);
    let inactive_tab = chrome_color(state, |colors| &colors.tab_inactive_background, 0x002B2623);
    let accent = chrome_color(state, |colors| &colors.accent, 0x00D78B55);
    let text = chrome_color(state, |colors| &colors.text, 0x00F1EDE9);
    let muted = chrome_color(state, |colors| &colors.muted_text, 0x00B8AEA6);
    let address = chrome_color(state, |colors| &colors.address_background, 0x00F5F3F1);
    let address_text = chrome_color(state, |colors| &colors.address_text, 0x00211D1B);
    paint_fill(hdc, 0, 0, width, 88, toolbar);
    paint_fill(hdc, 0, 39, width, 88, window);
    paint_fill(
        hdc,
        0,
        86,
        width,
        89,
        chrome_color(state, |colors| &colors.border, 0x00443B36),
    );

    let tab_area = (width - 134).max(0);
    let tab_width = (tab_area / state.tabs.len().max(1) as i32).clamp(120, 220);
    let active = state.browser.active_tab_id();
    for (index, (id, _)) in state.tabs.iter().enumerate() {
        let left = 10 + index as i32 * tab_width;
        let right = (left + tab_width - 6).min(width - 68);
        if right <= left {
            continue;
        }
        let selected = Some(*id) == active;
        paint_fill(
            hdc,
            left,
            7,
            right,
            36,
            if selected { active_tab } else { inactive_tab },
        );
        if selected {
            paint_fill(hdc, left + 8, 33, right - 8, 36, accent);
        }
        let title = state
            .browser
            .tabs()
            .tab(*id)
            .and_then(|tab| tab.page())
            .and_then(|page| page.title())
            .unwrap_or_else(|| format!("Nouvel onglet {}", index + 1));
        let label = title.chars().take(24).collect::<String>();
        paint_label(
            hdc,
            &label,
            left + 10,
            7,
            right - if state.tabs.len() > 1 { 34 } else { 10 },
            36,
            text,
            0,
        );
        if state.tabs.len() > 1 {
            paint_label(hdc, "×", right - 30, 7, right - 6, 36, muted, DT_CENTER);
        }
    }
    paint_fill(hdc, width - 108, 7, width - 62, 36, inactive_tab);
    paint_label(hdc, "⋯", width - 108, 7, width - 62, 36, text, DT_CENTER);
    paint_fill(hdc, width - 58, 7, width - 10, 36, inactive_tab);
    paint_label(hdc, "+", width - 58, 7, width - 10, 36, text, DT_CENTER);

    let can_go_back = state.browser.history().can_go_back();
    let can_go_forward = state.browser.history().can_go_forward();
    for (label, left, right, enabled) in [
        ("‹", 10, 44, can_go_back),
        ("›", 48, 82, can_go_forward),
        ("⟳", 86, 122, true),
    ] {
        paint_fill(
            hdc,
            left,
            48,
            right,
            80,
            if enabled { inactive_tab } else { window },
        );
        paint_label(
            hdc,
            label,
            left,
            48,
            right,
            80,
            if enabled { text } else { muted },
            DT_CENTER,
        );
    }
    let address_right = width - 170;
    paint_fill(hdc, 132, 48, address_right.max(240), 80, address);
    paint_fill(hdc, width - 122, 48, width - 10, 80, accent);
    let current_url = state.browser.current_url().map(ToString::to_string);
    let bookmarked = current_url
        .as_ref()
        .is_some_and(|url| state.bookmarks.iter().any(|bookmark| &bookmark.url == url));
    paint_fill(hdc, width - 166, 48, width - 132, 80, inactive_tab);
    paint_label(
        hdc,
        if bookmarked { "★" } else { "☆" },
        width - 166,
        48,
        width - 132,
        80,
        if bookmarked { accent } else { text },
        DT_CENTER,
    );
    paint_label(
        hdc,
        "Aller",
        width - 122,
        48,
        width - 10,
        80,
        address_text,
        DT_CENTER,
    );

    let bookmark_bar = chrome_color(state, |colors| &colors.toolbar_background, 0x00211D1B);
    paint_fill(hdc, 0, 90, width, 119, bookmark_bar);
    paint_fill(
        hdc,
        0,
        118,
        width,
        119,
        chrome_color(state, |colors| &colors.border, 0x00443B36),
    );
    if state.bookmarks.is_empty() {
        paint_label(
            hdc,
            "Ajoutez vos pages favorites avec ☆",
            14,
            91,
            width - 14,
            117,
            muted,
            0,
        );
    } else {
        let item_width = 154;
        let visible = ((width - 24).max(0) / (item_width + 8)) as usize;
        for (index, bookmark) in state.bookmarks.iter().take(visible).enumerate() {
            let left = 12 + index as i32 * (item_width + 8);
            paint_fill(hdc, left, 93, left + item_width, 115, inactive_tab);
            let label = bookmark.title.chars().take(20).collect::<String>();
            paint_label(
                hdc,
                &label,
                left + 8,
                93,
                left + item_width - 8,
                115,
                text,
                0,
            );
        }
    }
}

unsafe fn paint_home_screen(state: &AppState, hdc: Handle, width: i32, height: i32) {
    let top = 120;
    let background = chrome_color(state, |colors| &colors.window_background, 0x00181513);
    let card = chrome_color(state, |colors| &colors.tab_inactive_background, 0x00312B27);
    let accent = chrome_color(state, |colors| &colors.accent, 0x00D78B55);
    let text = chrome_color(state, |colors| &colors.text, 0x00F5F1ED);
    let muted = chrome_color(state, |colors| &colors.muted_text, 0x00B8AEA6);
    paint_fill(hdc, 0, top, width, height, background);
    let content_width = width.min(980);
    let left = (width - content_width) / 2;
    paint_fill(hdc, left, 150, left + content_width, height.min(560), card);
    paint_label(
        hdc,
        "ARCHIA",
        left + 30,
        190,
        left + content_width - 30,
        232,
        accent,
        DT_CENTER,
    );
    paint_label(
        hdc,
        "Le web, à votre façon.",
        left + 30,
        240,
        left + content_width - 30,
        300,
        text,
        DT_CENTER,
    );
    paint_label(
        hdc,
        "Un navigateur indépendant, construit autour du moteur Archia.",
        left + 30,
        304,
        left + content_width - 30,
        340,
        muted,
        DT_CENTER,
    );

    let gap = 16;
    let card_width = ((content_width - 80 - gap * 2) / 3).max(120);
    let card_top = 470;
    for (index, (title, detail)) in [
        ("Navigation", "Une interface simple et directe"),
        ("Indépendance", "Un moteur développé par Archia"),
        ("Confidentialité", "Pas de moteur tiers embarqué"),
    ]
    .iter()
    .enumerate()
    {
        let x = left + 40 + index as i32 * (card_width + gap);
        paint_fill(hdc, x, card_top, x + card_width, card_top + 112, card);
        paint_fill(hdc, x, card_top, x + card_width, card_top + 3, accent);
        paint_label(
            hdc,
            title,
            x + 10,
            card_top + 18,
            x + card_width - 10,
            card_top + 54,
            text,
            DT_CENTER,
        );
        paint_label(
            hdc,
            detail,
            x + 12,
            card_top + 55,
            x + card_width - 12,
            card_top + 94,
            muted,
            DT_CENTER,
        );
    }
    paint_label(
        hdc,
        "Saisissez une adresse dans la barre en haut pour commencer.",
        left + 20,
        620,
        left + content_width - 20,
        660,
        muted,
        DT_CENTER,
    );
}

unsafe fn layout_controls(state: &AppState, width: i32, height: i32) {
    let margin = 10;
    let button_y = 48;
    let button_h = 30;
    let _ = ShowWindow(state.back, SW_HIDE);
    let _ = ShowWindow(state.forward, SW_HIDE);
    let _ = ShowWindow(state.reload, SW_HIDE);
    let _ = ShowWindow(state.go, SW_HIDE);
    let _ = ShowWindow(state.new_tab, SW_HIDE);
    for (_, tab) in &state.tabs {
        let _ = ShowWindow(*tab, SW_HIDE);
    }
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
        margin + 122,
        button_y + 2,
        (width - 314).max(80),
        button_h - 4,
        1,
    );
    let _ = MoveWindow(state.go, width - 118, button_y, 108, button_h, 1);
    let home_width = (width - 120).clamp(220, 620);
    let home_left = (width - home_width - 82).max(20) / 2;
    let _ = MoveWindow(state.home_search, home_left, 378, home_width, 42, 1);
    let _ = MoveWindow(state.home_go, home_left + home_width + 10, 378, 72, 42, 1);
    let _ = MoveWindow(
        state.page,
        margin,
        120,
        (width - 2 * margin).max(100),
        (height - 132).max(100),
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
            state.address = create_control(hwnd, instance, "EDIT", "", ES_AUTOHSCROLL, ID_ADDRESS);
            state.home_search = create_control(
                hwnd,
                instance,
                "EDIT",
                "",
                ES_AUTOHSCROLL,
                ID_HOME_SEARCH,
            );
            state.home_go = create_control(hwnd, instance, "BUTTON", "Rechercher", 0, ID_HOME_GO);
            state.address_brush = CreateSolidBrush(chrome_color(
                state,
                |colors| &colors.address_background,
                0x00F5F3F1,
            ));
            state.address_proc = SetWindowLongPtrW(
                state.address,
                GWLP_WNDPROC,
                address_edit_proc as *const () as isize,
            );
            state.home_search_proc = SetWindowLongPtrW(
                state.home_search,
                GWLP_WNDPROC,
                address_edit_proc as *const () as isize,
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
            let _ = ShowWindow(initial_button, SW_HIDE);
            let _ = ShowWindow(state.back, SW_HIDE);
            let _ = ShowWindow(state.forward, SW_HIDE);
            let _ = ShowWindow(state.reload, SW_HIDE);
            let _ = ShowWindow(state.go, SW_HIDE);
            let _ = ShowWindow(state.new_tab, SW_HIDE);
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
            let _ = SendMessageW(state.home_search, 0x0030, 1, 0);
            let _ = ShowWindow(state.page, SW_HIDE);
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
                    if GetClientRect(hwnd, &mut client) != 0 {
                        let width = client.right.saturating_sub(client.left);
                        let height = client.bottom.saturating_sub(client.top);
                        paint_browser_chrome(state, hdc, width);
                        if state.browser.current_page().is_none() {
                            paint_home_screen(state, hdc, width, height);
                        }
                    }
                    if GetClientRect(hwnd, &mut client) != 0
                        && state.browser.current_page().is_some()
                    {
                        let width = client
                            .right
                            .saturating_sub(client.left)
                            .saturating_sub(36)
                            .max(1) as u32;
                        let height = client
                            .bottom
                            .saturating_sub(client.top)
                            .saturating_sub(132)
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
                                if state
                                    .browser
                                    .render_current_page_at(surface, state.scroll_y)
                                {
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
                                120,
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
                    width.saturating_sub(36).max(1) as u32,
                    height.saturating_sub(132).max(1) as u32,
                );
                if state.browser.relayout_current_page(viewport) {
                    state.surface_dirty = true;
                    update_scrollbar(state);
                    let _ = InvalidateRect(hwnd, ptr::null(), 1);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = app_state(hwnd) {
                let client_x = (l_param as u32 & 0xffff) as u16 as i16 as i32;
                let client_y = ((l_param as u32 >> 16) & 0xffff) as u16 as i16 as i32;
                let width = {
                    let mut client = Rect {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if GetClientRect(hwnd, &mut client) != 0 {
                        client.right.saturating_sub(client.left)
                    } else {
                        1180
                    }
                };
                if client_y < 42 {
                    let tab_area = (width - 134).max(0);
                    let tab_width = (tab_area / state.tabs.len().max(1) as i32).clamp(86, 220);
                    if client_x >= width - 58 && client_x <= width - 10 {
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
                            let _ = ShowWindow(button, SW_HIDE);
                            state.tabs.push((tab_id, button));
                            set_control_text(state.address, "");
                            refresh_page(state);
                        }
                    } else if client_x >= width - 108 && client_x <= width - 62 {
                        show_browser_menu(state, hwnd);
                    } else if (10..width - 108).contains(&client_x) {
                        let index = ((client_x - 10) / tab_width.max(1)) as usize;
                        if let Some((id, handle)) = state.tabs.get(index).copied() {
                            let tab_left = 10 + index as i32 * tab_width;
                            let close_clicked =
                                state.tabs.len() > 1 && client_x >= tab_left + tab_width - 36;
                            if close_clicked {
                                if state.browser.close_tab(id) {
                                    let _ = DestroyWindow(handle);
                                    state.tabs.remove(index);
                                    refresh_page(state);
                                }
                            } else {
                                state.browser.select_tab(id);
                                refresh_page(state);
                            }
                        }
                    }
                } else if (48..=82).contains(&client_x) && (45..=84).contains(&client_y) {
                    if state
                        .browser
                        .go_forward(&state.loader, current_viewport(state))
                        .is_ok()
                    {
                        refresh_page(state);
                    }
                } else if (86..=122).contains(&client_x) && (45..=84).contains(&client_y) {
                    if state
                        .browser
                        .reload(&state.loader, current_viewport(state))
                        .is_ok()
                    {
                        refresh_page(state);
                    }
                } else if client_x >= width - 166
                    && client_x <= width - 132
                    && (45..=84).contains(&client_y)
                {
                    toggle_current_bookmark(state);
                } else if client_x >= width - 122 && (45..=84).contains(&client_y) {
                    let address = read_control_text(state.address);
                    navigate(state, &address);
                } else if (10..=44).contains(&client_x) && (45..=84).contains(&client_y) {
                    if state
                        .browser
                        .go_back(&state.loader, current_viewport(state))
                        .is_ok()
                    {
                        refresh_page(state);
                    }
                } else if (90..=118).contains(&client_y) && client_x >= 12 {
                    let index = ((client_x - 12) / 162) as usize;
                    if let Some(bookmark) = state.bookmarks.get(index) {
                        let url = bookmark.url.clone();
                        navigate(state, &url);
                    }
                } else {
                    activate_link_at(state, hwnd, client_x, client_y);
                }
            }
            0
        }
        WM_KEYDOWN => {
            if let Some(state) = app_state(hwnd) {
                if handle_keyboard_shortcut(state, hwnd, w_param) {
                    return 0;
                }
            }
            DefWindowProcW(hwnd, message, w_param, l_param)
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
        WM_CTLCOLOREDIT => {
            if let Some(state) = app_state(hwnd) {
                if (l_param as Hwnd == state.address || l_param as Hwnd == state.home_search)
                    && !state.address_brush.is_null()
                {
                    let _ = SetTextColor(
                        w_param as Handle,
                        chrome_color(state, |colors| &colors.address_text, 0x00211D1B),
                    );
                    let _ = SetBkColor(
                        w_param as Handle,
                        chrome_color(state, |colors| &colors.address_background, 0x00F5F3F1),
                    );
                    return state.address_brush as Lresult;
                }
            }
            DefWindowProcW(hwnd, message, w_param, l_param)
        }
        WM_COMMAND => {
            let id = w_param & 0xffff;
            if let Some(state) = app_state(hwnd) {
                match id {
                    ID_GO => {
                        let address = read_control_text(state.address);
                        navigate(state, &address);
                    }
                    ID_HOME_GO => {
                        let address = read_control_text(state.home_search);
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
            if let Some(state) = app_state(hwnd) {
                if !state.address_brush.is_null() {
                    let _ = DeleteObject(state.address_brush);
                    state.address_brush = ptr::null_mut();
                }
            }
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
            home_search: ptr::null_mut(),
            home_go: ptr::null_mut(),
            address_brush: ptr::null_mut(),
            address_proc: 0,
            home_search_proc: 0,
            surface: None,
            surface_dirty: true,
            scroll_y: 0,
            dib_pixels: Vec::new(),
            tabs: Vec::new(),
            bookmarks: load_bookmarks(&bookmark_storage_path()),
            bookmarks_path: bookmark_storage_path(),
            themes: std::env::var_os("APPDATA")
                .map(|base| {
                    std::path::PathBuf::from(base)
                        .join("ArchiaBrowser")
                        .join("themes")
                })
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|base| base.join(".archia-browser").join("themes"))
                })
                .and_then(|directory| ThemeManager::load(directory).ok()),
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

#[cfg(test)]
mod gui_address_tests {
    use super::*;

    #[test]
    fn plain_words_use_the_search_provider() {
        assert_eq!(
            parse_address("cats").unwrap().to_string(),
            "https://www.google.com/search?q=cats"
        );
    }

    #[test]
    fn multi_word_queries_are_percent_encoded() {
        assert_eq!(
            parse_address("rust browser engine").unwrap().to_string(),
            "https://www.google.com/search?q=rust%20browser%20engine"
        );
    }

    #[test]
    fn dotted_hosts_still_open_directly() {
        assert_eq!(
            parse_address("example.org/path").unwrap().to_string(),
            "https://example.org/path"
        );
    }
}
