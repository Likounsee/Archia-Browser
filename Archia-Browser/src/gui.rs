//! Native Windows desktop frontend for Archia Browser.
//!
//! This first GUI milestone deliberately uses Win32 controls and the existing
//! Rust document engine. It does not embed Edge/WebView2 or another browser.

use std::{ffi::c_void, mem, ptr};

use archia_browser::{
    layout::LayoutViewport,
    net::{
        download_to_path,
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
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_COMMAND: u32 = 0x0111;
const WM_NCCREATE: u32 = 0x0081;
const WM_DESTROY: u32 = 0x0002;
const WM_KEYDOWN: u32 = 0x0100;
const VK_RETURN: usize = 0x0D;
const VK_L: usize = 0x4C;
const VK_T: usize = 0x54;
const VK_W: usize = 0x57;
const VK_R: usize = 0x52;
const VK_S: usize = 0x53;
const VK_D: usize = 0x44;
const VK_E: usize = 0x45;
const VK_C: usize = 0x43;
const VK_H: usize = 0x48;
const VK_J: usize = 0x4A;
const VK_K: usize = 0x4B;
const VK_O: usize = 0x4F;
const VK_I: usize = 0x49;
const VK_Y: usize = 0x59;
const VK_B: usize = 0x42;
const VK_F5: usize = 0x74;
const VK_F6: usize = 0x75;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SHIFT: i32 = 0x10;
const VK_LEFT: usize = 0x25;
const VK_TAB: usize = 0x09;
const VK_PRIOR: usize = 0x21;
const VK_NEXT: usize = 0x22;
const VK_1: usize = 0x31;
const VK_9: usize = 0x39;
const VK_DELETE: usize = 0x2E;
const VK_HOME: usize = 0x24;
const VK_RIGHT: usize = 0x27;
const MF_STRING: u32 = 0x0000;
const MF_GRAYED: u32 = 0x0001;
const MF_SEPARATOR: u32 = 0x0800;
const TPM_RETURNCMD: u32 = 0x0100;
const ID_MENU_RELOAD: u32 = 3001;
const ID_MENU_NEW_TAB: u32 = 3002;
const ID_MENU_CLOSE_TAB: u32 = 3003;
const ID_MENU_QUIT: u32 = 3004;
const ID_MENU_CLEAR_HISTORY: u32 = 3005;
const ID_MENU_OPEN_ALL_BOOKMARKS: u32 = 3006;
const ID_MENU_SEARCH_GOOGLE: u32 = 3007;
const ID_MENU_SEARCH_BING: u32 = 3008;
const ID_MENU_SEARCH_DDG: u32 = 3009;
const ID_MENU_SET_HOMEPAGE: u32 = 3014;
const ID_MENU_RESET_HOMEPAGE: u32 = 3015;
const ID_MENU_TOGGLE_SESSION: u32 = 3016;
const ID_MENU_DOWNLOAD_PAGE: u32 = 3017;
const ID_MENU_OPEN_DOWNLOADS: u32 = 3018;
const ID_MENU_RECENT_DOWNLOADS: u32 = 3025;
const ID_MENU_OPEN_RECENT_HISTORY: u32 = 3026;
const ID_MENU_SORT_BOOKMARKS_TITLE: u32 = 3027;
const ID_MENU_SORT_BOOKMARKS_DOMAIN: u32 = 3028;
const ID_MENU_DEDUPLICATE_BOOKMARKS: u32 = 3029;
const ID_MENU_COPY_BOOKMARKS: u32 = 3030;
const ID_MENU_COPY_HISTORY: u32 = 3031;
const ID_MENU_UA_ARCHIA: u32 = 3032;
const ID_MENU_UA_CHROMIUM: u32 = 3033;
const ID_MENU_UA_FIREFOX: u32 = 3034;
const ID_MENU_COPY_PAGE_TEXT: u32 = 3035;
const ID_MENU_COPY_PAGE_TITLE: u32 = 3036;
const ID_MENU_OPEN_EXTERNAL: u32 = 3037;
const ID_MENU_CLEAR_SITE_DATA: u32 = 3038;
const ID_MENU_COPY_OPEN_TABS: u32 = 3039;
const ID_MENU_HARD_RELOAD: u32 = 3040;
const ID_MENU_IMPORT_BOOKMARKS: u32 = 3041;
const ID_MENU_COPY_BOOKMARKS_HTML: u32 = 3042;
const ID_MENU_PASTE_AND_GO: u32 = 3043;
const ID_MENU_SEARCH_CLIPBOARD_NEW_TAB: u32 = 3044;
const ID_MENU_COPY_HISTORY_HTML: u32 = 3045;
const ID_MENU_COPY_OPEN_TABS_HTML: u32 = 3046;
const ID_MENU_OPEN_CLIPBOARD_LINKS: u32 = 3047;
const ID_MENU_COPY_BOOKMARKS_MARKDOWN: u32 = 3048;
const ID_MENU_COPY_HISTORY_MARKDOWN: u32 = 3049;
const ID_MENU_REOPEN_ALL_CLOSED_TABS: u32 = 3050;
const ID_MENU_COPY_CLOSED_TABS_MARKDOWN: u32 = 3051;
const ID_MENU_CLEAR_CLOSED_TABS: u32 = 3052;
const ID_MENU_COPY_OPEN_TABS_MARKDOWN: u32 = 3053;
const ID_MENU_COPY_OPEN_TABS_CSV: u32 = 3054;
const ID_MENU_CLOSED_TAB_BASE: u32 = 3800;
const MAX_MENU_CLOSED_TABS: usize = 20;
const MAX_IMPORTED_BOOKMARKS: usize = 5_000;
const MAX_CLIPBOARD_BYTES: usize = 2 * 1024 * 1024;
const ID_RECENT_DOWNLOAD_BASE: u32 = 3700;
const ID_RECENT_DOWNLOAD_REVEAL_BASE: u32 = 3720;
const ID_RECENT_DOWNLOAD_COPY_BASE: u32 = 3740;
const MAX_RECENT_DOWNLOADS: usize = 12;
const ID_MENU_REOPEN_CLOSED_TAB: u32 = 3019;
const ID_MENU_COPY_PAGE_URL: u32 = 3020;
const ID_MENU_BOOKMARK_PAGE: u32 = 3021;
const ID_LINK_OPEN_NEW_TAB: u32 = 3022;
const ID_LINK_COPY_URL: u32 = 3023;
const ID_LINK_DOWNLOAD: u32 = 3024;
const ID_MENU_THEME_ARCHIA: u32 = 3010;
const ID_MENU_THEME_CHROME: u32 = 3011;
const ID_MENU_THEME_FIREFOX: u32 = 3012;
const ID_MENU_THEME_EDGE: u32 = 3013;
const ID_MENU_HISTORY_BASE: u32 = 3200;
const ID_MENU_HISTORY_NEW_TAB_BASE: u32 = 3210;
const MAX_MENU_HISTORY_ITEMS: usize = 8;
const MAX_PERSISTED_HISTORY_ITEMS: usize = 500;
const MAX_RESTORED_TABS: usize = 20;
const ID_MENU_BOOKMARK_BASE: u32 = 3300;
const ID_MENU_BOOKMARK_NEW_TAB_BASE: u32 = 3320;
const MAX_MENU_BOOKMARK_ITEMS: usize = 16;
const ID_BOOKMARK_OPEN: u32 = 3501;
const ID_BOOKMARK_OPEN_NEW_TAB: u32 = 3502;
const ID_BOOKMARK_REMOVE: u32 = 3503;
const ID_TAB_CONTEXT_CLOSE: u32 = 3601;
const ID_TAB_CONTEXT_DUPLICATE: u32 = 3602;
const ID_TAB_CONTEXT_CLOSE_OTHERS: u32 = 3603;
const ID_TAB_CONTEXT_CLOSE_RIGHT: u32 = 3604;
const ID_TAB_CONTEXT_COPY_URL: u32 = 3605;
const ID_TAB_CONTEXT_RELOAD: u32 = 3606;
const ID_TAB_CONTEXT_CLOSE_LEFT: u32 = 3607;
const ID_TAB_CONTEXT_CLOSE_SAME_SITE: u32 = 3608;
const ID_TAB_CONTEXT_CLOSE_DUPLICATES: u32 = 3609;
const ID_TAB_CONTEXT_COPY_SITE_URLS: u32 = 3610;
const ID_TAB_CONTEXT_COPY_TITLE_URL: u32 = 3611;
const ID_TAB_CONTEXT_COPY_MARKDOWN_LINK: u32 = 3612;
const ID_TAB_CONTEXT_OPEN_EXTERNAL: u32 = 3613;
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
const DARK_ON_ACCENT: u32 = 0x000A1311;

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
    fn SetWindowRgn(hwnd: Hwnd, region: Handle, redraw: i32) -> i32;
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
    fn OpenClipboard(owner: Hwnd) -> i32;
    fn GetClipboardData(format: u32) -> Handle;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: Handle) -> Handle;
    fn CloseClipboard() -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn FillRect(hdc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn CreateSolidBrush(color: u32) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn CreateRoundRectRgn(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: i32,
        height: i32,
    ) -> Handle;
    fn FillRgn(hdc: Handle, region: Handle, brush: Handle) -> i32;
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

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: Hwnd,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_cmd: i32,
    ) -> Handle;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(module_name: *const u16) -> Hinstance;
    fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    fn GlobalLock(memory: Handle) -> *mut c_void;
    fn GlobalUnlock(memory: Handle) -> i32;
    fn GlobalSize(memory: Handle) -> usize;
    fn GlobalFree(memory: Handle) -> Handle;
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

    fn from_id(value: &str) -> Option<Self> {
        match value {
            "google" => Some(Self::Google),
            "bing" => Some(Self::Bing),
            "duckduckgo" => Some(Self::DuckDuckGo),
            _ => None,
        }
    }

    fn search_url(self, query: &str) -> String {
        let query = encode_search_query(query);
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

    fn from_id(value: &str) -> Option<Self> {
        match value {
            "archia" => Some(Self::Archia),
            "chromium" => Some(Self::ChromiumCompatible),
            "firefox" => Some(Self::FirefoxCompatible),
            _ => None,
        }
    }

    fn header(self, version: &str) -> String {
        match self {
            Self::Archia => format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ArchiaBrowser/{version}"
            ),
            Self::ChromiumCompatible => format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 ArchiaBrowser/{version}"
            ),
            Self::FirefoxCompatible => format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0 ArchiaBrowser/{version}"
            ),
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
            homepage: "https://www.google.com/".into(),
            restore_session: true,
            user_agent: UserAgentProfile::Archia,
        }
    }
}

impl BrowserSettings {
    fn load(path: &std::path::Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "search_provider" => {
                    if let Some(provider) = SearchProvider::from_id(value.trim()) {
                        settings.search_provider = provider;
                    }
                }
                "homepage" => {
                    if let Ok(url) = Url::parse(value.trim()) {
                        if matches!(url.scheme(), "http" | "https") {
                            settings.homepage = url.to_string();
                        }
                    }
                }
                "restore_session" => match value.trim() {
                    "true" => settings.restore_session = true,
                    "false" => settings.restore_session = false,
                    _ => {}
                },
                "user_agent" => {
                    if let Some(profile) = UserAgentProfile::from_id(value.trim()) {
                        settings.user_agent = profile;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = format!(
            "search_provider={}\nhomepage={}\nrestore_session={}\nuser_agent={}\n",
            self.search_provider.id(),
            self.homepage.replace(['\r', '\n'], ""),
            self.restore_session,
            self.user_agent.id()
        );
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, contents)?;
        match std::fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) if path.exists() => {
                std::fs::remove_file(path)?;
                std::fs::rename(&temporary, path).map_err(|_| error)
            }
            Err(error) => Err(error),
        }
    }
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
    address_font: Handle,
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
    history_path: std::path::PathBuf,
    session_path: std::path::PathBuf,
    persistent_history: Vec<Bookmark>,
    startup_session: Vec<String>,
    settings: BrowserSettings,
    settings_path: std::path::PathBuf,
    closed_tabs: Vec<Option<String>>,
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
    let mut bookmarks = contents
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
        .collect::<Vec<_>>();
    deduplicate_bookmarks(&mut bookmarks);
    bookmarks
}

fn bookmark_host_key(url: &str) -> String {
    Url::parse(url)
        .ok()
        .map(|parsed| parsed.host().to_ascii_lowercase())
        .unwrap_or_else(|| url.to_ascii_lowercase())
}

fn sort_bookmarks_by_title(bookmarks: &mut [Bookmark]) {
    bookmarks.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.url.cmp(&right.url))
    });
}

fn sort_bookmarks_by_domain(bookmarks: &mut [Bookmark]) {
    bookmarks.sort_by_key(|bookmark| {
        (
            bookmark_host_key(&bookmark.url),
            bookmark.title.to_lowercase(),
            bookmark.url.clone(),
        )
    });
}

fn deduplicate_bookmarks(bookmarks: &mut Vec<Bookmark>) {
    let mut seen = std::collections::HashSet::new();
    bookmarks.retain(|bookmark| seen.insert(bookmark.url.clone()));
}

fn parse_imported_bookmarks(text: &str) -> Vec<Bookmark> {
    if text.contains("](") {
        return parse_markdown_bookmarks(text);
    }
    if text.to_ascii_lowercase().contains("<a ") {
        return parse_netscape_bookmarks(text);
    }
    let mut imported = Vec::new();
    for line in text.lines().take(MAX_IMPORTED_BOOKMARKS) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((first, second)) = line.split_once('\t') else {
            if let Ok(url) = Url::parse(line) {
                if matches!(url.scheme(), "http" | "https") {
                    imported.push(Bookmark {
                        title: url.host().to_owned(),
                        url: url.to_string(),
                    });
                }
            }
            continue;
        };
        let first = first.trim();
        let second = second.trim();
        let (title, raw_url) =
            if Url::parse(first).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
                (second, first)
            } else if Url::parse(second).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
                (first, second)
            } else {
                continue;
            };
        let Ok(url) = Url::parse(raw_url) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        imported.push(Bookmark {
            title: if title.is_empty() {
                url.host().to_owned()
            } else {
                title.to_owned()
            },
            url: url.to_string(),
        });
    }
    deduplicate_bookmarks(&mut imported);
    imported
}

fn parse_markdown_bookmarks(text: &str) -> Vec<Bookmark> {
    let mut imported = Vec::new();
    for line in text.lines().take(MAX_IMPORTED_BOOKMARKS) {
        let line = line.trim().trim_start_matches(['-', '*']).trim();
        let Some(label_start) = line.find('[') else {
            continue;
        };
        let Some(label_end) = line[label_start + 1..].find("](") else {
            continue;
        };
        let label_end = label_start + 1 + label_end;
        let raw_label = &line[label_start + 1..label_end];
        let target = line[label_end + 2..].trim();
        let target = target.strip_suffix(')').unwrap_or(target).trim();
        let target = target
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or(target);
        let Ok(url) = Url::parse(target) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let title = raw_label
            .replace("\\[", "[")
            .replace("\\]", "]")
            .replace("\\\\", "\\");
        imported.push(Bookmark {
            title: if title.trim().is_empty() {
                url.host().to_owned()
            } else {
                title
            },
            url: url.to_string(),
        });
    }
    deduplicate_bookmarks(&mut imported);
    imported
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

fn parse_netscape_bookmarks(text: &str) -> Vec<Bookmark> {
    let lower = text.to_ascii_lowercase();
    let mut imported = Vec::new();
    let mut cursor = 0;
    while imported.len() < MAX_IMPORTED_BOOKMARKS {
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
        let (raw_url, _) = if let Some(value) = attribute.strip_prefix('"') {
            let Some(end) = value.find('"') else {
                cursor = tag_end + 1;
                continue;
            };
            (&value[..end], end + 2)
        } else if let Some(value) = attribute.strip_prefix('\'') {
            let Some(end) = value.find('\'') else {
                cursor = tag_end + 1;
                continue;
            };
            (&value[..end], end + 2)
        } else {
            let end = attribute
                .find(char::is_whitespace)
                .unwrap_or(attribute.len());
            (&attribute[..end], end)
        };
        let Some(close_relative) = lower[tag_end + 1..].find("</a>") else {
            break;
        };
        let close_start = tag_end + 1 + close_relative;
        let title = text[tag_end + 1..close_start]
            .replace("<b>", "")
            .replace("</b>", "")
            .replace("<B>", "")
            .replace("</B>", "");
        let title = decode_bookmark_html_text(&title);
        if let Ok(url) = Url::parse(&decode_bookmark_html_text(raw_url)) {
            if matches!(url.scheme(), "http" | "https") {
                imported.push(Bookmark {
                    title: if title.is_empty() {
                        url.host().to_owned()
                    } else {
                        title
                    },
                    url: url.to_string(),
                });
            }
        }
        cursor = close_start + 4;
    }
    deduplicate_bookmarks(&mut imported);
    imported
}

fn escape_bookmark_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn export_bookmarks_markdown(bookmarks: &[Bookmark]) -> String {
    bookmarks
        .iter()
        .filter_map(|bookmark| {
            let url = Url::parse(&bookmark.url).ok()?;
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            Some(format!(
                "- {}",
                markdown_link(&bookmark.title, &url.to_string())
            ))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn export_bookmarks_html(bookmarks: &[Bookmark]) -> String {
    let entries = bookmarks
        .iter()
        .map(|bookmark| {
            format!(
                "<DT><A HREF=\"{}\">{}</A>",
                escape_bookmark_html(&bookmark.url),
                escape_bookmark_html(&bookmark.title)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Archia Browser Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n{entries}\n</DL><p>\n"
    )
}

fn explicit_url_scheme(value: &str) -> Option<&str> {
    let (scheme, remainder) = value.split_once(':')?;
    let mut characters = scheme.chars();
    if !characters.next()?.is_ascii_alphabetic()
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
        || (!remainder.is_empty()
            && remainder
                .chars()
                .all(|character| character.is_ascii_digit()))
    {
        return None;
    }
    Some(scheme)
}

fn clipboard_navigation_url(value: &str, provider: SearchProvider) -> Option<Url> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
        return None;
    }
    if let Some(scheme) = explicit_url_scheme(value) {
        if !matches!(scheme, "http" | "https") || !value[scheme.len() + 1..].starts_with("//") {
            return None;
        }
    }
    let url = parse_address_with_provider(value, provider).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn markdown_link(title: &str, url: &str) -> String {
    let label = if title.trim().is_empty() { url } else { title };
    let escaped = label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("[{escaped}](<{url}>)")
}

fn export_bookmarks_text(bookmarks: &[Bookmark]) -> String {
    bookmarks
        .iter()
        .map(|bookmark| format!("{}\t{}", bookmark.title, bookmark.url))
        .collect::<Vec<_>>()
        .join("\n")
}

fn export_open_tabs_text(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .map(|(title, url)| format!("{title}\t{url}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn export_open_tabs_csv(entries: &[(String, String)]) -> String {
            "title,url\r\n\"Docs, \"\"draft\"\"\",\"https://docs.example/path\"\r\n\"Second\",\"https://example.org/\""
    for (title, url) in entries {
        let Ok(parsed) = Url::parse(url) else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        let title = title.replace('"', "\"\"");
        let url = parsed.to_string().replace('"', "\"\"");
        rows.push(format!("\"{title}\",\"{url}\""));
    }
    if rows.len() == 1 {
        String::new()
    } else {
        rows.join("\r\n")
    }
}

fn export_open_tabs_markdown(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .filter_map(|(title, url)| {
            let parsed = Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            Some(format!("- {}", markdown_link(title, &parsed.to_string())))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn export_open_tabs_html(entries: &[(String, String)]) -> String {
    let bookmarks = entries
        .iter()
        .filter_map(|(title, url)| {
            let parsed = Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            Some(Bookmark {
                title: if title.trim().is_empty() {
                    parsed.host().to_owned()
                } else {
                    title.clone()
                },
                url: parsed.to_string(),
            })
        })
        .collect::<Vec<_>>();
    export_bookmarks_html(&bookmarks)
}

fn browser_data_directory() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.join(".archia-browser"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("ArchiaBrowser")
}

fn downloads_directory() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("Downloads")
}

fn safe_download_filename(value: &str) -> String {
    let mut name = value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    while name.ends_with([' ', '.']) {
        name.pop();
    }
    if name.is_empty()
        || matches!(
            name.split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str(),
            "CON" | "PRN" | "AUX" | "NUL"
        )
    {
        name.insert(0, '_');
    }
    name.chars().take(180).collect()
}

fn unique_download_destination(directory: &std::path::Path, filename: &str) -> std::path::PathBuf {
    let original = std::path::Path::new(filename);
    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
    let extension = original
        .extension()
        .map(|value| value.to_string_lossy().to_string());
    for index in 0..10_000_u32 {
        let candidate_name = if index == 0 {
            filename.to_owned()
        } else if let Some(extension) = &extension {
            format!("{stem} ({index}).{extension}")
        } else {
            format!("{stem} ({index})")
        };
        let candidate = directory.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{filename}.download"))
}

fn settings_storage_path() -> std::path::PathBuf {
    browser_data_directory().join("settings.ini")
}

fn history_storage_path() -> std::path::PathBuf {
    browser_data_directory().join("history.tsv")
}

fn session_storage_path() -> std::path::PathBuf {
    browser_data_directory().join("session.txt")
}

fn load_persistent_history(path: &std::path::Path) -> Vec<Bookmark> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut history = contents
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
        .rev()
        .take(MAX_PERSISTED_HISTORY_ITEMS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    deduplicate_history_keep_latest(&mut history);
    history
}

fn deduplicate_history_keep_latest(history: &mut Vec<Bookmark>) {
    let mut seen = std::collections::HashSet::new();
    history.reverse();
    history.retain(|entry| seen.insert(entry.url.clone()));
    history.reverse();
}

fn load_session(path: &std::path::Path) -> Vec<String> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let url = Url::parse(line).ok()?;
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
        })
        .take(MAX_RESTORED_TABS)
        .collect()
}

fn save_persistent_history(state: &AppState) {
    if let Some(parent) = state.history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let contents = state
        .persistent_history
        .iter()
        .map(|entry| {
            format!(
                "{}\t{}",
                entry.url.replace(['\t', '\r', '\n'], " "),
                entry.title.replace(['\t', '\r', '\n'], " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&state.history_path, contents);
}

fn record_history(state: &mut AppState) {
    let Some(url) = state.browser.current_url().map(ToString::to_string) else {
        return;
    };
    let title = state
        .browser
        .current_page()
        .and_then(|page| page.title())
        .unwrap_or_else(|| url.clone());
    state.persistent_history.retain(|entry| entry.url != url);
    state.persistent_history.push(Bookmark { title, url });
    if state.persistent_history.len() > MAX_PERSISTED_HISTORY_ITEMS {
        let excess = state.persistent_history.len() - MAX_PERSISTED_HISTORY_ITEMS;
        state.persistent_history.drain(..excess);
    }
    save_persistent_history(state);
}

fn save_session(state: &AppState) {
    if !state.settings.restore_session {
        let _ = std::fs::remove_file(&state.session_path);
        return;
    }
    if let Some(parent) = state.session_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let urls = state
        .tabs
        .iter()
        .filter_map(|(id, _)| {
            state
                .browser
                .tabs()
                .tab(*id)
                .and_then(|tab| tab.page())
                .and_then(|page| page.url())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let _ = std::fs::write(&state.session_path, urls.join("\n"));
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
                bookmark.url.replace(['\t', '\r', '\n'], " "),
                bookmark.title.replace(['\t', '\r', '\n'], " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&state.bookmarks_path, contents);
}

unsafe fn open_downloads_folder(state: &mut AppState) {
    let directory = downloads_directory();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        let caption = wide(&format!(
            "Ouverture des téléchargements impossible : {error}"
        ));
        let _ = SetWindowTextW(state.window, caption.as_ptr());
        return;
    }
    let path = wide(&directory.to_string_lossy());
    let operation = wide("open");
    let result = ShellExecuteW(
        state.window,
        operation.as_ptr(),
        path.as_ptr(),
        ptr::null(),
        ptr::null(),
        SW_SHOW,
    );
    if (result as isize) <= 32 {
        let caption = wide("Windows n’a pas pu ouvrir le dossier Téléchargements — Archia Browser");
        let _ = SetWindowTextW(state.window, caption.as_ptr());
    }
}

unsafe fn show_recent_downloads(state: &mut AppState, hwnd: Hwnd) {
    let directory = downloads_directory();
    let mut files = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
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
    files.truncate(MAX_RECENT_DOWNLOADS);

    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let heading = wide(&format!("Téléchargements récents ({})", files.len()));
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, heading.as_ptr());
    if files.is_empty() {
        let empty = wide("Aucun fichier téléchargé");
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, empty.as_ptr());
    } else {
        for (index, (_, path)) in files.iter().enumerate() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let short_name = name.chars().take(48).collect::<String>();
            let open_label = wide(&format!("Ouvrir — {short_name}"));
            let reveal_label = wide(&format!("Afficher dans l'Explorateur — {short_name}"));
            let copy_label = wide(&format!("Copier le chemin — {short_name}"));
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ID_RECENT_DOWNLOAD_BASE + index as u32) as usize,
                open_label.as_ptr(),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ID_RECENT_DOWNLOAD_REVEAL_BASE + index as u32) as usize,
                reveal_label.as_ptr(),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ID_RECENT_DOWNLOAD_COPY_BASE + index as u32) as usize,
                copy_label.as_ptr(),
            );
        }
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    let folder = wide("Ouvrir le dossier Téléchargements");
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        ID_MENU_OPEN_DOWNLOADS as usize,
        folder.as_ptr(),
    );
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
    if selected == ID_MENU_OPEN_DOWNLOADS {
        open_downloads_folder(state);
    } else if selected >= ID_RECENT_DOWNLOAD_BASE
        && selected < ID_RECENT_DOWNLOAD_BASE + files.len() as u32
    {
        let index = (selected - ID_RECENT_DOWNLOAD_BASE) as usize;
        if let Some((_, path)) = files.get(index) {
            let operation = wide("open");
            let target = wide(&path.to_string_lossy());
            let _ = ShellExecuteW(
                hwnd,
                operation.as_ptr(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOW,
            );
        }
    } else if selected >= ID_RECENT_DOWNLOAD_REVEAL_BASE
        && selected < ID_RECENT_DOWNLOAD_REVEAL_BASE + files.len() as u32
    {
        let index = (selected - ID_RECENT_DOWNLOAD_REVEAL_BASE) as usize;
        if let Some((_, path)) = files.get(index) {
            let operation = wide("open");
            let target = wide("explorer.exe");
            let arguments = wide(&format!("/select,\"{}\"", path.display()));
            let _ = ShellExecuteW(
                hwnd,
                operation.as_ptr(),
                target.as_ptr(),
                arguments.as_ptr(),
                ptr::null(),
                SW_SHOW,
            );
        }
    } else if selected >= ID_RECENT_DOWNLOAD_COPY_BASE
        && selected < ID_RECENT_DOWNLOAD_COPY_BASE + files.len() as u32
    {
        let index = (selected - ID_RECENT_DOWNLOAD_COPY_BASE) as usize;
        if let Some((_, path)) = files.get(index) {
            let _ = copy_text_to_clipboard(hwnd, &path.to_string_lossy());
        }
    }
}

unsafe fn download_current_page(state: &mut AppState) {
    let Some(url) = state.browser.current_url().cloned() else {
        return;
    };
    download_url(state, url);
}

unsafe fn download_url(state: &mut AppState, url: Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    let directory = downloads_directory();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        let caption = wide(&format!("Échec du téléchargement : {error}"));
        let _ = SetWindowTextW(state.window, caption.as_ptr());
        return;
    }
    let pending = directory.join(format!(".archia-pending-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&pending);
    let mut request = Request::new(url);
    request.headers.insert("accept".into(), "*/*".into());
    request.headers.insert(
        "user-agent".into(),
        state.settings.user_agent.header(state.browser.version()),
    );
    let transport = HttpTransport::new().with_limits(64 * 1024 * 1024, 64 * 1024);
    match download_to_path(&transport, &request, &pending) {
        Ok(result) => {
            let filename = safe_download_filename(&result.suggested_filename);
            let destination = unique_download_destination(&directory, &filename);
            match std::fs::hard_link(&pending, &destination) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&pending);
                    let caption = wide(&format!(
                        "Téléchargement terminé : {} — Archia Browser",
                        destination.display()
                    ));
                    let _ = SetWindowTextW(state.window, caption.as_ptr());
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&pending);
                    let caption = wide(&format!(
                        "Échec de l'enregistrement : {error} — Archia Browser"
                    ));
                    let _ = SetWindowTextW(state.window, caption.as_ptr());
                }
            }
        }
        Err(error) => {
            let _ = std::fs::remove_file(&pending);
            let caption = wide(&format!(
                "Téléchargement impossible : {error:?} — Archia Browser"
            ));
            let _ = SetWindowTextW(state.window, caption.as_ptr());
        }
    }
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

fn looks_like_host(input: &str) -> bool {
    let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
    authority.starts_with('[') && authority.contains(']')
        || authority == "localhost"
        || authority.starts_with("localhost:")
        || authority.parse::<std::net::IpAddr>().is_ok()
        || authority.contains('.')
}

fn parse_address(input: &str) -> Result<Url, archia_browser::net::UrlError> {
    parse_address_with_provider(input, SearchProvider::Google)
}

fn parse_address_with_provider(
    input: &str,
    provider: SearchProvider,
) -> Result<Url, archia_browser::net::UrlError> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else if input.chars().any(char::is_whitespace) || !looks_like_host(input) {
        provider.search_url(input)
    } else {
        format!("https://{input}")
    };
    Url::parse(&address)
}

fn security_badge_label(url: Option<&Url>) -> &'static str {
    match url.map(Url::scheme) {
        Some("https") => "HTTPS",
        Some("http") => "HTTP",
        _ => "WEB",
    }
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
    if address.trim().is_empty() {
        return;
    }
    let url = match parse_address_with_provider(address, state.settings.search_provider) {
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
        state.settings.user_agent.header(state.browser.version()),
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
        Ok(_) => {
            record_history(state);
            save_session(state);
            refresh_page(state);
        }
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

unsafe fn paint_round_fill(
    hdc: Handle,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    color: u32,
    radius: i32,
) {
    if right <= left || bottom <= top {
        return;
    }
    let region = CreateRoundRectRgn(left, top, right + 1, bottom + 1, radius * 2, radius * 2);
    let brush = CreateSolidBrush(color);
    if !region.is_null() && !brush.is_null() {
        let _ = FillRgn(hdc, region, brush);
    }
    if !region.is_null() {
        let _ = DeleteObject(region);
    }
    if !brush.is_null() {
        let _ = DeleteObject(brush);
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

unsafe fn open_all_bookmarks(state: &mut AppState, hwnd: Hwnd) {
    let capacity = MAX_GUI_TABS
        .saturating_sub(state.tabs.len())
        .saturating_add(1)
        .min(MAX_RESTORED_TABS);
    let urls = state
        .bookmarks
        .iter()
        .take(capacity)
        .map(|bookmark| bookmark.url.clone())
        .collect::<Vec<_>>();
    for (index, url) in urls.iter().enumerate() {
        if index > 0 {
            create_new_tab(state, hwnd);
        }
        navigate(state, url);
    }
}

unsafe fn open_recent_history_in_tabs(state: &mut AppState, hwnd: Hwnd) {
    let capacity = MAX_GUI_TABS
        .saturating_sub(state.tabs.len())
        .saturating_add(1)
        .min(MAX_MENU_HISTORY_ITEMS);
    let urls = state
        .persistent_history
        .iter()
        .rev()
        .take(capacity)
        .map(|entry| entry.url.clone())
        .collect::<Vec<_>>();
    for (index, url) in urls.iter().enumerate() {
        if index > 0 {
            create_new_tab(state, hwnd);
        }
        navigate(state, url);
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
    save_session(state);
    set_control_text(state.address, "");
    refresh_page(state);
    let _ = SetFocus(state.address);
}

unsafe fn close_tab_at(state: &mut AppState, index: usize) {
    if state.tabs.len() <= 1 {
        return;
    }
    let Some((id, handle)) = state.tabs.get(index).copied() else {
        return;
    };
    let closed_url = state
        .browser
        .tabs()
        .tab(id)
        .and_then(|tab| tab.page())
        .and_then(|page| page.url())
        .map(ToString::to_string);
    if state.browser.close_tab(id) {
        if state.closed_tabs.len() == 20 {
            state.closed_tabs.remove(0);
        }
        state.closed_tabs.push(closed_url);
        let _ = DestroyWindow(handle);
        state.tabs.remove(index);
        save_session(state);
        refresh_page(state);
    }
}

unsafe fn close_active_tab(state: &mut AppState) {
    let Some(index) = state
        .browser
        .active_tab_id()
        .and_then(|id| state.tabs.iter().position(|(tab_id, _)| *tab_id == id))
    else {
        return;
    };
    close_tab_at(state, index);
}

fn tab_indices_to_close(active_index: usize, tab_count: usize, close_left: bool) -> Vec<usize> {
    if active_index >= tab_count {
        return Vec::new();
    }
    if close_left {
        (0..active_index).rev().collect()
    } else {
        (active_index + 1..tab_count).rev().collect()
    }
}

fn tab_indices_for_same_site(hosts: &[Option<String>], active_index: usize) -> Vec<usize> {
    let Some(target_host) = hosts.get(active_index).and_then(Option::as_deref) else {
        return Vec::new();
    };
    hosts
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(index, host)| {
            (index != active_index
                && host
                    .as_deref()
                    .is_some_and(|host| host.eq_ignore_ascii_case(target_host)))
            .then_some(index)
        })
        .collect()
}

fn urls_for_same_site(
    hosts: &[Option<String>],
    urls: &[Option<String>],
    active_index: usize,
) -> Vec<String> {
    let Some(target_host) = hosts.get(active_index).and_then(Option::as_deref) else {
        return Vec::new();
    };
    hosts
        .iter()
        .zip(urls.iter())
        .filter_map(|(host, url)| {
            (host
                .as_deref()
                .is_some_and(|host| host.eq_ignore_ascii_case(target_host)))
            .then(|| url.as_ref().cloned())
            .flatten()
        })
        .collect()
}

fn tab_indices_for_duplicate_urls(urls: &[Option<String>], active_index: usize) -> Vec<usize> {
    let Some(target_url) = urls.get(active_index).and_then(Option::as_deref) else {
        return Vec::new();
    };
    urls.iter()
        .enumerate()
        .rev()
        .filter_map(|(index, url)| {
            (index != active_index && url.as_deref() == Some(target_url)).then_some(index)
        })
        .collect()
}

fn tab_indices_to_close_except(active_index: usize, tab_count: usize) -> Vec<usize> {
    if active_index >= tab_count {
        return Vec::new();
    }
    (0..tab_count)
        .rev()
        .filter(|index| *index != active_index)
        .collect()
}

unsafe fn close_other_tabs(state: &mut AppState) {
    let Some(active_id) = state.browser.active_tab_id() else {
        return;
    };
    let Some(active_index) = state.tabs.iter().position(|(id, _)| *id == active_id) else {
        return;
    };
    for index in tab_indices_to_close_except(active_index, state.tabs.len()) {
        close_tab_at(state, index);
    }
    if let Some(index) = state.tabs.iter().position(|(id, _)| *id == active_id) {
        select_tab_index(state, index);
    }
}

unsafe fn close_tabs_on_side(state: &mut AppState, close_left: bool) {
    let Some(active_id) = state.browser.active_tab_id() else {
        return;
    };
    let Some(active_index) = state.tabs.iter().position(|(id, _)| *id == active_id) else {
        return;
    };
    let indices = tab_indices_to_close(active_index, state.tabs.len(), close_left);
    for index in indices {
        close_tab_at(state, index);
    }
    if let Some(index) = state.tabs.iter().position(|(id, _)| *id == active_id) {
        select_tab_index(state, index);
    }
}

unsafe fn reopen_closed_tab(state: &mut AppState, hwnd: Hwnd) {
    let Some(closed_url) = state.closed_tabs.pop() else {
        return;
    };
    create_new_tab(state, hwnd);
    if let Some(url) = closed_url {
        navigate(state, &url);
    }
}

unsafe fn reopen_all_closed_tabs(state: &mut AppState, hwnd: Hwnd) {
    while !state.closed_tabs.is_empty() && state.tabs.len() < MAX_GUI_TABS {
        reopen_closed_tab(state, hwnd);
    }
}

unsafe fn reopen_closed_tab_at(state: &mut AppState, hwnd: Hwnd, recent_index: usize) {
    if state.tabs.len() >= MAX_GUI_TABS {
        return;
    }
    let Some(storage_index) = closed_tab_storage_index(state.closed_tabs.len(), recent_index)
    else {
        return;
    };
    let closed_url = state.closed_tabs.remove(storage_index);
    create_new_tab(state, hwnd);
    if let Some(url) = closed_url {
        navigate(state, &url);
    }
}

fn closed_tab_storage_index(closed_tab_count: usize, recent_index: usize) -> Option<usize> {
    (recent_index < closed_tab_count).then(|| closed_tab_count - 1 - recent_index)
}

fn clear_closed_tabs(closed_tabs: &mut Vec<Option<String>>) -> usize {
    let removed = closed_tabs.len();
    closed_tabs.clear();
    removed
}

fn closed_tabs_as_bookmarks(closed_tabs: &[Option<String>]) -> Vec<Bookmark> {
    closed_tabs
        .iter()
        .filter_map(|url| {
            let url = url.as_ref()?;
            let parsed = Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            Some(Bookmark {
                title: parsed.host().to_owned(),
                url: parsed.to_string(),
            })
        })
        .collect()
}

unsafe fn select_tab_index(state: &mut AppState, index: usize) {
    if let Some((id, _)) = state.tabs.get(index) {
        state.browser.select_tab(*id);
        refresh_page(state);
    }
}

unsafe fn handle_keyboard_shortcut(state: &mut AppState, hwnd: Hwnd, key: Wparam) -> bool {
    let control = GetKeyState(VK_CONTROL) < 0;
    let alt = GetKeyState(VK_MENU) < 0;
    if control {
        if alt && GetKeyState(VK_SHIFT) < 0 && key == VK_T {
            reopen_all_closed_tabs(state, hwnd);
            return true;
        }
        if alt && GetKeyState(VK_SHIFT) < 0 && key == VK_R {
            let markdown = export_bookmarks_markdown(&closed_tabs_as_bookmarks(&state.closed_tabs));
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
            return true;
        }
        if alt && GetKeyState(VK_SHIFT) < 0 && key == 0x58 {
            clear_closed_tabs(&mut state.closed_tabs);
            return true;
        }
        if alt && (key == VK_LEFT || key == VK_RIGHT) {
            close_tabs_on_side(state, key == VK_LEFT);
            return true;
        }
        if alt && key == VK_D {
            if let Some(url) = current_page_url(state) {
                open_link_in_new_tab(state, hwnd, &url);
            }
            return true;
        }
        if alt && key == VK_C {
            copy_current_page_title_and_url(state, hwnd);
            return true;
        }
        if alt && key == 0x4D && GetKeyState(VK_SHIFT) < 0 {
            copy_open_tabs_markdown(state, hwnd);
            return true;
        }
        if alt && key == 0x4D {
            copy_current_page_markdown_link(state, hwnd);
            return true;
        }
        if key == VK_W && GetKeyState(VK_SHIFT) < 0 && alt {
            close_other_tabs(state);
            return true;
        }
        if key == VK_D && GetKeyState(VK_SHIFT) < 0 {
            if let Some(url) = current_page_url(state) {
                if state.tabs.len() < MAX_GUI_TABS {
                    create_new_tab(state, hwnd);
                    navigate(state, &url);
                }
            }
            return true;
        }
        if (key == VK_TAB || key == VK_PRIOR || key == VK_NEXT) && !state.tabs.is_empty() {
            let active_index = state
                .browser
                .active_tab_id()
                .and_then(|id| state.tabs.iter().position(|(tab_id, _)| *tab_id == id))
                .unwrap_or(0);
            let previous = GetKeyState(VK_SHIFT) < 0 || key == VK_PRIOR;
            let next_index = if previous {
                (active_index + state.tabs.len() - 1) % state.tabs.len()
            } else {
                (active_index + 1) % state.tabs.len()
            };
            select_tab_index(state, next_index);
            return true;
        }
        if (VK_1..=VK_9).contains(&key) && !state.tabs.is_empty() {
            let requested = key - VK_1;
            let index = if requested == 8 {
                state.tabs.len() - 1
            } else {
                requested
            };
            select_tab_index(state, index);
            return true;
        }
        if key == VK_C && GetKeyState(VK_SHIFT) < 0 {
            if let Some(url) = current_page_url(state) {
                let _ = copy_text_to_clipboard(hwnd, &url);
            }
            return true;
        }
        if key == 0x58 && GetKeyState(VK_SHIFT) < 0 {
            copy_current_page_text(state, hwnd);
            return true;
        }
        if key == VK_E && GetKeyState(VK_SHIFT) < 0 {
            if let Some(url) = current_page_url(state) {
                open_url_externally(hwnd, &url);
            }
            return true;
        }
        if key == VK_K && GetKeyState(VK_SHIFT) < 0 {
            if let Some(url) = current_page_url(state) {
                open_link_in_new_tab(state, hwnd, &url);
            }
            return true;
        }
        if key == VK_H && GetKeyState(VK_SHIFT) < 0 {
            open_recent_history_in_tabs(state, hwnd);
            return true;
        }
        if key == VK_J && GetKeyState(VK_SHIFT) < 0 {
            show_recent_downloads(state, hwnd);
            return true;
        }
        if key == VK_O && GetKeyState(VK_SHIFT) < 0 {
            open_all_bookmarks(state, hwnd);
            return true;
        }
        if key == VK_I && GetKeyState(VK_SHIFT) < 0 {
            import_bookmarks_from_clipboard(state, hwnd);
            return true;
        }
        if key == 0x56 && GetKeyState(VK_SHIFT) < 0 {
            navigate_from_clipboard(state, hwnd, false);
            return true;
        }
        if key == 0x46 && GetKeyState(VK_SHIFT) < 0 {
            navigate_from_clipboard(state, hwnd, true);
            return true;
        }
        if key == 0x4D && GetKeyState(VK_SHIFT) < 0 {
            open_links_from_clipboard(state, hwnd);
            return true;
        }
        if key == 0x55 && alt {
            copy_open_tabs_html(state, hwnd);
            return true;
        }
        if key == 0x47 && GetKeyState(VK_SHIFT) < 0 {
            let html = export_bookmarks_html(&state.persistent_history);
            if !state.persistent_history.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &html);
            }
            return true;
        }
        if alt && key == VK_H && GetKeyState(VK_SHIFT) < 0 {
            let markdown = export_bookmarks_markdown(&state.persistent_history);
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
            return true;
        }
        if alt && key == VK_B && GetKeyState(VK_SHIFT) < 0 {
            let markdown = export_bookmarks_markdown(&state.bookmarks);
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
            return true;
        }
        if key == VK_B && GetKeyState(VK_SHIFT) < 0 {
            let html = export_bookmarks_html(&state.bookmarks);
            if !state.bookmarks.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &html);
            }
            return true;
        }
        if key == VK_O && GetKeyState(VK_SHIFT) < 0 {
            open_all_bookmarks(state, hwnd);
            return true;
        }
        if key == VK_Y && GetKeyState(VK_SHIFT) < 0 {
            let text = export_bookmarks_text(&state.persistent_history);
            if !text.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &text);
            }
            return true;
        }
        if key == 0x55 && GetKeyState(VK_SHIFT) < 0 {
            let entries = state
                .tabs
                .iter()
                .filter_map(|(id, _)| {
                    let tab = state.browser.tabs().tab(*id)?;
                    let page = tab.page()?;
                    Some((
                        page.title().unwrap_or_else(|| "(sans titre)".to_owned()),
                        page.url()?.to_string(),
                    ))
                })
                .collect::<Vec<_>>();
            let text = export_open_tabs_text(&entries);
            if !text.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &text);
            }
            return true;
        }
        match key {
            VK_L | VK_K => {
                let _ = SetFocus(state.address);
                let _ = SendMessageW(state.address, 0x00B1, 0, -1); // EM_SETSEL
                return true;
            }
            VK_T => {
                if GetKeyState(VK_SHIFT) < 0 {
                    reopen_closed_tab(state, hwnd);
                } else {
                    create_new_tab(state, hwnd);
                }
                let _ = SetFocus(state.address);
                return true;
            }
            VK_W => {
                close_active_tab(state);
                return true;
            }
            VK_R => {
                if GetKeyState(VK_SHIFT) < 0 {
                    let _ = state.loader.clear_cache();
                }
                if state
                    .browser
                    .reload(&state.loader, current_viewport(state))
                    .is_ok()
                {
                    refresh_page(state);
                }
                return true;
            }
            VK_S => {
                download_current_page(state);
                return true;
            }
            VK_D => {
                toggle_current_bookmark(state);
                return true;
            }
            VK_H => {
                show_browser_menu(state, hwnd);
                return true;
            }
            VK_J => {
                open_downloads_folder(state);
                return true;
            }
            VK_HOME => {
                let homepage = state.settings.homepage.clone();
                navigate(state, &homepage);
                return true;
            }
            VK_DELETE if GetKeyState(VK_SHIFT) < 0 => {
                state.persistent_history.clear();
                save_persistent_history(state);
                return true;
            }
            _ => {}
        }
    }
    if key == VK_F6 {
        let _ = SetFocus(state.address);
        let _ = SendMessageW(state.address, 0x00B1, 0, -1); // EM_SETSEL
        return true;
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
    if alt && key == VK_HOME {
        let homepage = state.settings.homepage.clone();
        navigate(state, &homepage);
        return true;
    }
    if alt && key == VK_LEFT {
        if state
            .browser
            .go_back(&state.loader, current_viewport(state))
            .is_ok()
        {
            record_history(state);
            save_session(state);
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
            record_history(state);
            save_session(state);
            refresh_page(state);
        }
        return true;
    }
    false
}

unsafe fn show_tab_context_menu(state: &mut AppState, hwnd: Hwnd, index: usize) {
    let Some((tab_id, _)) = state.tabs.get(index).copied() else {
        return;
    };
    let duplicate_url = state
        .browser
        .tabs()
        .tab(tab_id)
        .and_then(|tab| tab.page())
        .and_then(|page| page.url())
        .map(ToString::to_string);
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_TAB_CONTEXT_CLOSE, "Fermer l'onglet"),
        (ID_TAB_CONTEXT_DUPLICATE, "Dupliquer l'onglet"),
        (ID_TAB_CONTEXT_RELOAD, "Recharger cet onglet"),
        (ID_TAB_CONTEXT_COPY_URL, "Copier l'adresse de l'onglet"),
        (ID_TAB_CONTEXT_CLOSE_OTHERS, "Fermer les autres onglets"),
        (ID_TAB_CONTEXT_CLOSE_RIGHT, "Fermer les onglets à droite"),
        (ID_TAB_CONTEXT_CLOSE_LEFT, "Fermer les onglets à gauche"),
        (
            ID_TAB_CONTEXT_CLOSE_SAME_SITE,
            "Fermer les autres onglets de ce site",
        ),
        (
            ID_TAB_CONTEXT_CLOSE_DUPLICATES,
            "Fermer les onglets avec la même adresse",
        ),
        (
            ID_TAB_CONTEXT_COPY_SITE_URLS,
            "Copier les adresses de tous les onglets de ce site",
        ),
        (
            ID_TAB_CONTEXT_COPY_TITLE_URL,
            "Copier le titre et l'adresse de cet onglet",
        ),
        (
            ID_TAB_CONTEXT_COPY_MARKDOWN_LINK,
            "Copier le lien Markdown de cet onglet",
        ),
        (
            ID_TAB_CONTEXT_OPEN_EXTERNAL,
            "Ouvrir dans le navigateur externe",
        ),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
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
        ID_TAB_CONTEXT_CLOSE => close_tab_at(state, index),
        ID_TAB_CONTEXT_RELOAD => {
            state.browser.select_tab(tab_id);
            if state
                .browser
                .reload(&state.loader, current_viewport(state))
                .is_ok()
            {
                refresh_page(state);
            }
        }
        ID_TAB_CONTEXT_COPY_URL => {
            if let Some(url) = duplicate_url.as_deref() {
                let _ = copy_text_to_clipboard(hwnd, url);
            }
        }
        ID_TAB_CONTEXT_COPY_MARKDOWN_LINK => {
            if let Some(page) = state.browser.tabs().tab(tab_id).and_then(|tab| tab.page()) {
                if let Some(url) = page.url() {
                    let title = page.title().unwrap_or_default();
                    let link = markdown_link(&title, &url.to_string());
                    let _ = copy_text_to_clipboard(hwnd, &link);
                }
            }
        }
        ID_TAB_CONTEXT_OPEN_EXTERNAL => {
            if let Some(url) = duplicate_url.as_deref() {
                open_url_externally(hwnd, url);
            }
        }
        ID_TAB_CONTEXT_COPY_TITLE_URL => {
            if let Some(page) = state.browser.tabs().tab(tab_id).and_then(|tab| tab.page()) {
                if let Some(url) = page.url() {
                    let title = page
                        .title()
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| url.to_string());
                    let _ = copy_text_to_clipboard(hwnd, &format!("{title}\r\n{url}"));
                }
            }
        }
        ID_TAB_CONTEXT_COPY_SITE_URLS => {
            let hosts = state
                .tabs
                .iter()
                .map(|(id, _)| {
                    state
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.page())
                        .and_then(|page| page.url())
                        .map(|url| url.host().to_owned())
                })
                .collect::<Vec<_>>();
            let urls = state
                .tabs
                .iter()
                .map(|(id, _)| {
                    state
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.page())
                        .and_then(|page| page.url())
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>();
            let site_urls = urls_for_same_site(&hosts, &urls, index);
            if !site_urls.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &site_urls.join("\r\n"));
            }
        }
        ID_TAB_CONTEXT_CLOSE_DUPLICATES => {
            let urls = state
                .tabs
                .iter()
                .map(|(id, _)| {
                    state
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.page())
                        .and_then(|page| page.url())
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>();
            for duplicate_index in tab_indices_for_duplicate_urls(&urls, index) {
                close_tab_at(state, duplicate_index);
            }
            if let Some(active_index) = state.tabs.iter().position(|(id, _)| *id == tab_id) {
                select_tab_index(state, active_index);
            }
        }
        ID_TAB_CONTEXT_CLOSE_SAME_SITE => {
            let hosts = state
                .tabs
                .iter()
                .map(|(id, _)| {
                    state
                        .browser
                        .tabs()
                        .tab(*id)
                        .and_then(|tab| tab.page())
                        .and_then(|page| page.url())
                        .map(|url| url.host().to_owned())
                })
                .collect::<Vec<_>>();
            for other_index in tab_indices_for_same_site(&hosts, index) {
                close_tab_at(state, other_index);
            }
            if let Some(active_index) = state.tabs.iter().position(|(id, _)| *id == tab_id) {
                select_tab_index(state, active_index);
            }
        }
        ID_TAB_CONTEXT_CLOSE_RIGHT | ID_TAB_CONTEXT_CLOSE_LEFT => {
            let close_left = selected == ID_TAB_CONTEXT_CLOSE_LEFT;
            for other_index in tab_indices_to_close(index, state.tabs.len(), close_left) {
                close_tab_at(state, other_index);
            }
            if let Some(active_index) = state.tabs.iter().position(|(id, _)| *id == tab_id) {
                select_tab_index(state, active_index);
            }
        }
        ID_TAB_CONTEXT_DUPLICATE => {
            if let Some(url) = duplicate_url {
                create_new_tab(state, hwnd);
                navigate(state, &url);
            }
        }
        ID_TAB_CONTEXT_CLOSE_OTHERS => {
            for other_index in (0..state.tabs.len()).rev() {
                if state
                    .tabs
                    .get(other_index)
                    .is_some_and(|(id, _)| *id != tab_id)
                {
                    close_tab_at(state, other_index);
                }
            }
            if let Some((active_index, _)) = state
                .tabs
                .iter()
                .enumerate()
                .find(|(_, (id, _))| *id == tab_id)
            {
                select_tab_index(state, active_index);
            }
        }
        _ => {}
    }
}

unsafe fn current_page_url(state: &AppState) -> Option<String> {
    state
        .browser
        .active_tab_id()
        .and_then(|id| state.browser.tabs().tab(id))
        .and_then(|tab| tab.page())
        .and_then(|page| page.url())
        .map(ToString::to_string)
}

const MAX_PAGE_CLIPBOARD_BYTES: usize = 2 * 1024 * 1024;

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let boundary = value
        .char_indices()
        .take_while(|(index, character)| index + character.len_utf8() <= max_bytes)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    value.truncate(boundary);
}

unsafe fn copy_current_page_text(state: &AppState, hwnd: Hwnd) {
    let Some(page) = state.browser.current_page() else {
        return;
    };
    let mut text = page.document.visible_text_content();
    truncate_utf8(&mut text, MAX_PAGE_CLIPBOARD_BYTES);
    if !text.trim().is_empty() {
        let _ = copy_text_to_clipboard(hwnd, &text);
    }
}

unsafe fn copy_current_page_title(state: &AppState, hwnd: Hwnd) {
    if let Some(title) = state.browser.current_page().and_then(|page| page.title()) {
        if !title.is_empty() {
            let _ = copy_text_to_clipboard(hwnd, &title);
        }
    }
}

unsafe fn copy_current_page_title_and_url(state: &AppState, hwnd: Hwnd) {
    let Some(page) = state.browser.current_page() else {
        return;
    };
    let Some(url) = page.url() else {
        return;
    };
    let title = page
        .title()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| url.to_string());
    let _ = copy_text_to_clipboard(hwnd, &format!("{title}\t{url}"));
}

unsafe fn copy_current_page_markdown_link(state: &AppState, hwnd: Hwnd) {
    let Some(page) = state.browser.current_page() else {
        return;
    };
    let Some(url) = page.url() else {
        return;
    };
    let title = page.title().unwrap_or_default();
    let link = markdown_link(&title, &url.to_string());
    let _ = copy_text_to_clipboard(hwnd, &link);
}

unsafe fn open_url_externally(hwnd: Hwnd, value: &str) {
    let Ok(url) = Url::parse(value) else {
        return;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    let operation = wide("open");
    let target = wide(&url.to_string());
    let result = ShellExecuteW(
        hwnd,
        operation.as_ptr(),
        target.as_ptr(),
        ptr::null(),
        ptr::null(),
        SW_SHOW,
    );
    if (result as isize) <= 32 {
        let caption =
            wide("Impossible d’ouvrir cette adresse dans le navigateur externe — Archia Browser");
        let _ = SetWindowTextW(hwnd, caption.as_ptr());
    }
}

unsafe fn read_text_from_clipboard(hwnd: Hwnd) -> Option<String> {
    const CF_UNICODETEXT: u32 = 13;
    if OpenClipboard(hwnd) == 0 {
        return None;
    }
    let memory = GetClipboardData(CF_UNICODETEXT);
    if memory.is_null() {
        let _ = CloseClipboard();
        return None;
    }
    let byte_len = GlobalSize(memory).min(MAX_CLIPBOARD_BYTES);
    if byte_len < std::mem::size_of::<u16>() {
        let _ = CloseClipboard();
        return None;
    }
    let source = GlobalLock(memory) as *const u16;
    if source.is_null() {
        let _ = CloseClipboard();
        return None;
    }
    let max_units = byte_len / std::mem::size_of::<u16>();
    let mut length = 0;
    while length < max_units && *source.add(length) != 0 {
        length += 1;
    }
    let value = String::from_utf16_lossy(std::slice::from_raw_parts(source, length));
    let _ = GlobalUnlock(memory);
    let _ = CloseClipboard();
    Some(value)
}

unsafe fn navigate_from_clipboard(state: &mut AppState, hwnd: Hwnd, new_tab: bool) {
    let Some(text) = read_text_from_clipboard(hwnd) else {
        return;
    };
    let Some(url) = clipboard_navigation_url(&text, state.settings.search_provider) else {
        return;
    };
    if new_tab {
        if state.tabs.len() >= MAX_GUI_TABS {
            return;
        }
        create_new_tab(state, hwnd);
    }
    navigate(state, &url.to_string());
}

unsafe fn open_links_from_clipboard(state: &mut AppState, hwnd: Hwnd) {
    let Some(text) = read_text_from_clipboard(hwnd) else {
        return;
    };
    let links = parse_imported_bookmarks(&text)
        .into_iter()
        .take(MAX_RESTORED_TABS.min(MAX_GUI_TABS))
        .map(|bookmark| bookmark.url)
        .collect::<Vec<_>>();
    if links.is_empty() || state.tabs.is_empty() {
        return;
    }
    let capacity = state
        .tabs
        .len()
        .saturating_add(links.len().saturating_sub(1))
        .min(MAX_GUI_TABS);
    for (index, url) in links
        .into_iter()
        .take(capacity.saturating_sub(state.tabs.len()).saturating_add(1))
        .enumerate()
    {
        if index > 0 {
            create_new_tab(state, hwnd);
        }
        navigate(state, &url);
    }
}

unsafe fn copy_open_tabs_html(state: &AppState, hwnd: Hwnd) {
    let entries = state
        .tabs
        .iter()
        .filter_map(|(id, _)| {
            let page = state.browser.tabs().tab(*id)?.page()?;
            Some((
                page.title().unwrap_or_else(|| "(sans titre)".to_owned()),
                page.url()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    if !entries.is_empty() {
        let html = export_open_tabs_html(&entries);
        let _ = copy_text_to_clipboard(hwnd, &html);
    }
}

unsafe fn copy_open_tabs_markdown(state: &AppState, hwnd: Hwnd) {
    let entries = state
        .tabs
        .iter()
        .filter_map(|(id, _)| {
            let page = state.browser.tabs().tab(*id)?.page()?;
            Some((
                page.title().unwrap_or_else(|| "(sans titre)".to_owned()),
                page.url()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let markdown = export_open_tabs_markdown(&entries);
    if !markdown.is_empty() {
        let _ = copy_text_to_clipboard(hwnd, &markdown);
    }
}

unsafe fn import_bookmarks_from_clipboard(state: &mut AppState, hwnd: Hwnd) {
    let Some(text) = read_text_from_clipboard(hwnd) else {
        return;
    };
    let imported = parse_imported_bookmarks(&text);
    if imported.is_empty() {
        return;
    }
    let previous_count = state.bookmarks.len();
    state.bookmarks.extend(imported);
    deduplicate_bookmarks(&mut state.bookmarks);
    if state.bookmarks.len() != previous_count {
        save_bookmarks(state);
        let _ = InvalidateRect(hwnd, ptr::null(), 1);
    }
}

unsafe fn copy_text_to_clipboard(hwnd: Hwnd, value: &str) -> bool {
    const GMEM_MOVEABLE: u32 = 0x0002;
    const CF_UNICODETEXT: u32 = 13;
    let text = wide(value);
    let memory = GlobalAlloc(
        GMEM_MOVEABLE,
        text.len().saturating_mul(std::mem::size_of::<u16>()),
    );
    if memory.is_null() {
        return false;
    }
    let destination = GlobalLock(memory) as *mut u16;
    if destination.is_null() {
        let _ = GlobalFree(memory);
        return false;
    }
    ptr::copy_nonoverlapping(text.as_ptr(), destination, text.len());
    let _ = GlobalUnlock(memory);
    if OpenClipboard(hwnd) == 0 {
        let _ = GlobalFree(memory);
        return false;
    }
    let success = EmptyClipboard() != 0 && !SetClipboardData(CF_UNICODETEXT, memory).is_null();
    let _ = CloseClipboard();
    if !success {
        let _ = GlobalFree(memory);
    }
    success
}

unsafe fn link_url_at(state: &AppState, client_x: i32, client_y: i32) -> Option<String> {
    let viewport = current_viewport(state);
    let content_x = client_x.saturating_sub(10);
    let content_y = client_y.saturating_sub(120);
    if content_x < 0
        || content_y < 0
        || content_x as u32 >= viewport.width
        || content_y as u32 >= viewport.height
    {
        return None;
    }
    let document_y = content_y.saturating_add(state.scroll_y.min(i32::MAX as u32) as i32);
    state.browser.current_page().and_then(|page| {
        let link = page.link_at(content_x, document_y)?;
        let href = link.link_href()?.trim();
        let url = page.resolve_reference(href).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        Some(url.to_string())
    })
}

unsafe fn open_link_in_new_tab(state: &mut AppState, hwnd: Hwnd, url: &str) {
    if state.tabs.len() >= MAX_GUI_TABS {
        return;
    }
    create_new_tab(state, hwnd);
    navigate(state, url);
}

unsafe fn show_link_context_menu(state: &mut AppState, hwnd: Hwnd, url: String) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_LINK_OPEN_NEW_TAB, "Ouvrir le lien dans un nouvel onglet"),
        (ID_LINK_COPY_URL, "Copier l’adresse du lien"),
        (ID_LINK_DOWNLOAD, "Télécharger le lien"),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
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
        ID_LINK_OPEN_NEW_TAB if state.tabs.len() < MAX_GUI_TABS => {
            open_link_in_new_tab(state, hwnd, &url);
        }
        ID_LINK_COPY_URL => {
            let _ = copy_text_to_clipboard(hwnd, &url);
        }
        ID_LINK_DOWNLOAD => {
            if let Ok(parsed) = Url::parse(&url) {
                download_url(state, parsed);
            }
        }
        _ => {}
    }
}

unsafe fn show_page_context_menu(state: &mut AppState, hwnd: Hwnd) {
    let Some(url) = current_page_url(state) else {
        return;
    };
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_MENU_COPY_PAGE_URL, "Copier l’adresse de la page"),
        (ID_MENU_COPY_PAGE_TITLE, "Copier le titre de la page"),
        (ID_MENU_COPY_PAGE_TEXT, "Copier le texte visible de la page"),
        (
            ID_MENU_OPEN_EXTERNAL,
            "Ouvrir dans le navigateur par défaut",
        ),
        (
            ID_MENU_BOOKMARK_PAGE,
            if state.bookmarks.iter().any(|bookmark| bookmark.url == url) {
                "Retirer des favoris"
            } else {
                "Ajouter aux favoris"
            },
        ),
        (ID_MENU_RELOAD, "Recharger la page"),
        (ID_MENU_DOWNLOAD_PAGE, "Télécharger la page"),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
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
        ID_MENU_COPY_PAGE_URL => {
            let _ = copy_text_to_clipboard(hwnd, &url);
        }
        ID_MENU_COPY_PAGE_TITLE => copy_current_page_title(state, hwnd),
        ID_MENU_COPY_PAGE_TEXT => copy_current_page_text(state, hwnd),
        ID_MENU_OPEN_EXTERNAL => open_url_externally(hwnd, &url),
        ID_MENU_BOOKMARK_PAGE => toggle_current_bookmark(state),
        ID_MENU_RELOAD => navigate(state, &url),
        ID_MENU_DOWNLOAD_PAGE => download_current_page(state),
        _ => {}
    }
}

unsafe fn show_bookmark_context_menu(state: &mut AppState, hwnd: Hwnd, index: usize) {
    let Some(bookmark) = state.bookmarks.get(index) else {
        return;
    };
    let url = bookmark.url.clone();
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_BOOKMARK_OPEN, "Ouvrir le favori"),
        (ID_BOOKMARK_OPEN_NEW_TAB, "Ouvrir dans un nouvel onglet"),
        (ID_BOOKMARK_REMOVE, "Supprimer des favoris"),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
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
        ID_BOOKMARK_OPEN => navigate(state, &url),
        ID_BOOKMARK_OPEN_NEW_TAB => {
            create_new_tab(state, hwnd);
            navigate(state, &url);
        }
        ID_BOOKMARK_REMOVE if index < state.bookmarks.len() => {
            state.bookmarks.remove(index);
            save_bookmarks(state);
            let _ = InvalidateRect(hwnd, ptr::null(), 1);
        }
        _ => {}
    }
}

unsafe fn show_browser_menu(state: &mut AppState, hwnd: Hwnd) {
    let history_entries = state.persistent_history.clone();
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (ID_MENU_RELOAD, "Recharger"),
        (ID_MENU_HARD_RELOAD, "Recharger sans utiliser le cache"),
        (ID_MENU_NEW_TAB, "Nouvel onglet"),
        (ID_MENU_CLOSE_TAB, "Fermer l'onglet"),
        (ID_MENU_CLEAR_HISTORY, "Effacer l'historique"),
        (ID_MENU_CLEAR_SITE_DATA, "Effacer les cookies et le cache"),
        (ID_MENU_OPEN_ALL_BOOKMARKS, "Ouvrir tous les favoris"),
        (
            ID_MENU_REOPEN_ALL_CLOSED_TABS,
            "Rouvrir tous les onglets récemment fermés",
        ),
        (
            ID_MENU_COPY_CLOSED_TABS_MARKDOWN,
            "Copier les onglets fermés au format Markdown",
        ),
        (
            ID_MENU_CLEAR_CLOSED_TABS,
            "Vider la liste des onglets fermés",
        ),
        (ID_MENU_SEARCH_GOOGLE, "Recherche : Google"),
        (ID_MENU_SEARCH_BING, "Recherche : Bing"),
        (ID_MENU_SEARCH_DDG, "Recherche : DuckDuckGo"),
        (
            ID_MENU_SET_HOMEPAGE,
            "Définir la page actuelle comme accueil",
        ),
        (ID_MENU_RESET_HOMEPAGE, "Réinitialiser la page d'accueil"),
        (ID_MENU_DOWNLOAD_PAGE, "Télécharger la page actuelle"),
        (ID_MENU_OPEN_DOWNLOADS, "Ouvrir le dossier Téléchargements"),
        (ID_MENU_RECENT_DOWNLOADS, "Téléchargements récents…"),
        (
            ID_MENU_OPEN_RECENT_HISTORY,
            "Ouvrir les pages récentes dans des onglets",
        ),
        (ID_MENU_SORT_BOOKMARKS_TITLE, "Trier les favoris par nom"),
        (ID_MENU_SORT_BOOKMARKS_DOMAIN, "Trier les favoris par site"),
        (
            ID_MENU_DEDUPLICATE_BOOKMARKS,
            "Supprimer les favoris en double",
        ),
        (ID_MENU_COPY_BOOKMARKS, "Copier tous les favoris (texte)"),
        (
            ID_MENU_COPY_BOOKMARKS_HTML,
            "Copier les favoris au format HTML (Netscape)",
        ),
        (
            ID_MENU_COPY_BOOKMARKS_MARKDOWN,
            "Copier les favoris au format Markdown",
        ),
        (
            ID_MENU_COPY_HISTORY_MARKDOWN,
            "Copier l’historique au format Markdown",
        ),
        (
            ID_MENU_PASTE_AND_GO,
            "Coller et ouvrir l’adresse ou rechercher le texte copié",
        ),
        (
            ID_MENU_SEARCH_CLIPBOARD_NEW_TAB,
            "Rechercher le texte copié dans un nouvel onglet",
        ),
        (
            ID_MENU_COPY_HISTORY_HTML,
            "Copier l’historique au format HTML (Netscape)",
        ),
        (
            ID_MENU_COPY_OPEN_TABS_HTML,
            "Exporter les onglets ouverts en HTML (Netscape)",
        ),
        (
            ID_MENU_COPY_OPEN_TABS_MARKDOWN,
            "Copier les onglets ouverts au format Markdown",
        ),
        (
            ID_MENU_COPY_OPEN_TABS_CSV,
            "Copier les onglets ouverts au format CSV",
        ),
        (
            ID_MENU_OPEN_CLIPBOARD_LINKS,
            "Ouvrir les liens du presse-papiers dans des onglets",
        ),
        (
            ID_MENU_IMPORT_BOOKMARKS,
            "Importer les favoris depuis le presse-papiers",
        ),
        (ID_MENU_COPY_HISTORY, "Copier l'historique"),
        (
            ID_MENU_COPY_OPEN_TABS,
            "Copier les adresses de tous les onglets",
        ),
        (
            ID_MENU_UA_ARCHIA,
            if state.settings.user_agent == UserAgentProfile::Archia {
                "✓ Agent utilisateur : Archia"
            } else {
                "Agent utilisateur : Archia"
            },
        ),
        (
            ID_MENU_UA_CHROMIUM,
            if state.settings.user_agent == UserAgentProfile::ChromiumCompatible {
                "✓ Agent utilisateur : compatible Chromium"
            } else {
                "Agent utilisateur : compatible Chromium"
            },
        ),
        (
            ID_MENU_UA_FIREFOX,
            if state.settings.user_agent == UserAgentProfile::FirefoxCompatible {
                "✓ Agent utilisateur : compatible Firefox"
            } else {
                "Agent utilisateur : compatible Firefox"
            },
        ),
        (
            ID_MENU_TOGGLE_SESSION,
            if state.settings.restore_session {
                "Désactiver la restauration de session"
            } else {
                "Activer la restauration de session"
            },
        ),
        (ID_MENU_THEME_ARCHIA, "Theme : Archia"),
        (ID_MENU_THEME_CHROME, "Theme : Chrome"),
        (ID_MENU_THEME_FIREFOX, "Theme : Firefox"),
        (ID_MENU_THEME_EDGE, "Theme : Edge"),
        (ID_MENU_QUIT, "Quitter Archia Browser"),
    ] {
        let label = wide(label);
        let _ = AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
    }
    let reopen_label = wide("Rouvrir le dernier onglet fermé");
    let reopen_flags = MF_STRING
        | if state.closed_tabs.is_empty() {
            MF_GRAYED
        } else {
            0
        };
    let _ = AppendMenuW(
        menu,
        reopen_flags,
        ID_MENU_REOPEN_CLOSED_TAB as usize,
        reopen_label.as_ptr(),
    );
    for (recent_index, entry) in state
        .closed_tabs
        .iter()
        .rev()
        .take(MAX_MENU_CLOSED_TABS)
        .enumerate()
    {
        let label = entry
            .as_deref()
            .and_then(|value| Url::parse(value).ok())
            .map(|url| format!("{} — {}", url.host(), url.path()))
            .unwrap_or_else(|| "Adresse indisponible".to_owned());
        let label = label.chars().take(58).collect::<String>();
        let label = wide(&format!("Rouvrir : {label}"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            (ID_MENU_CLOSED_TAB_BASE + recent_index as u32) as usize,
            label.as_ptr(),
        );
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    let history_heading = wide("Pages récemment visitées");
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, history_heading.as_ptr());
    for (index, entry) in history_entries
        .iter()
        .rev()
        .take(MAX_MENU_HISTORY_ITEMS)
        .enumerate()
    {
        let title = entry.title.as_str();
        let short_title = title.chars().take(42).collect::<String>();
        let label = wide(&format!("{} — {}", index + 1, short_title));
        let new_tab_label = wide(&format!("↗ {}", short_title));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            (ID_MENU_HISTORY_BASE as usize) + index,
            label.as_ptr(),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            (ID_MENU_HISTORY_NEW_TAB_BASE as usize) + index,
            new_tab_label.as_ptr(),
        );
    }
    if !state.bookmarks.is_empty() {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        let heading = wide("Favoris");
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, heading.as_ptr());
        for (index, bookmark) in state
            .bookmarks
            .iter()
            .take(MAX_MENU_BOOKMARK_ITEMS)
            .enumerate()
        {
            let short_title = bookmark.title.chars().take(38).collect::<String>();
            let label = wide(&format!("★ {}", short_title));
            let new_tab_label = wide(&format!("↗ {}", short_title));
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ID_MENU_BOOKMARK_BASE as usize) + index,
                label.as_ptr(),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                (ID_MENU_BOOKMARK_NEW_TAB_BASE as usize) + index,
                new_tab_label.as_ptr(),
            );
        }
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
        ID_MENU_HARD_RELOAD => {
            let _ = state.loader.clear_cache();
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
        ID_MENU_REOPEN_CLOSED_TAB => reopen_closed_tab(state, hwnd),
        ID_MENU_REOPEN_ALL_CLOSED_TABS => reopen_all_closed_tabs(state, hwnd),
        ID_MENU_CLEAR_CLOSED_TABS => {
            clear_closed_tabs(&mut state.closed_tabs);
        }
        id if (ID_MENU_CLOSED_TAB_BASE..ID_MENU_CLOSED_TAB_BASE + MAX_MENU_CLOSED_TABS as u32)
            .contains(&id) =>
        {
            reopen_closed_tab_at(state, hwnd, (id - ID_MENU_CLOSED_TAB_BASE) as usize);
        }
        ID_MENU_COPY_CLOSED_TABS_MARKDOWN => {
            let markdown = export_bookmarks_markdown(&closed_tabs_as_bookmarks(&state.closed_tabs));
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
        }
        ID_MENU_CLEAR_HISTORY => {
            state.persistent_history.clear();
            save_persistent_history(state);
        }
        ID_MENU_CLEAR_SITE_DATA => {
            let _ = state.loader.clear_cookies_and_cache();
        }
        ID_MENU_SEARCH_GOOGLE | ID_MENU_SEARCH_BING | ID_MENU_SEARCH_DDG => {
            state.settings.search_provider = match selected {
                ID_MENU_SEARCH_GOOGLE => SearchProvider::Google,
                ID_MENU_SEARCH_BING => SearchProvider::Bing,
                _ => SearchProvider::DuckDuckGo,
            };
            let _ = state.settings.save(&state.settings_path);
        }
        ID_MENU_SET_HOMEPAGE => {
            if let Some(url) = state.browser.current_url() {
                state.settings.homepage = url.to_string();
                let _ = state.settings.save(&state.settings_path);
            }
        }
        ID_MENU_RESET_HOMEPAGE => {
            state.settings.homepage = BrowserSettings::default().homepage;
            let _ = state.settings.save(&state.settings_path);
        }
        ID_MENU_DOWNLOAD_PAGE => download_current_page(state),
        ID_MENU_OPEN_DOWNLOADS => open_downloads_folder(state),
        ID_MENU_RECENT_DOWNLOADS => show_recent_downloads(state, hwnd),
        ID_MENU_OPEN_RECENT_HISTORY => open_recent_history_in_tabs(state, hwnd),
        ID_MENU_SORT_BOOKMARKS_TITLE => {
            sort_bookmarks_by_title(&mut state.bookmarks);
            save_bookmarks(state);
            let _ = InvalidateRect(hwnd, ptr::null(), 1);
        }
        ID_MENU_SORT_BOOKMARKS_DOMAIN => {
            sort_bookmarks_by_domain(&mut state.bookmarks);
            save_bookmarks(state);
            let _ = InvalidateRect(hwnd, ptr::null(), 1);
        }
        ID_MENU_DEDUPLICATE_BOOKMARKS => {
            let previous_count = state.bookmarks.len();
            deduplicate_bookmarks(&mut state.bookmarks);
            if state.bookmarks.len() != previous_count {
                save_bookmarks(state);
                let _ = InvalidateRect(hwnd, ptr::null(), 1);
            }
        }
        ID_MENU_COPY_HISTORY_MARKDOWN => {
            let markdown = export_bookmarks_markdown(&state.persistent_history);
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
        }
        ID_MENU_COPY_BOOKMARKS_MARKDOWN => {
            let markdown = export_bookmarks_markdown(&state.bookmarks);
            if !markdown.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &markdown);
            }
        }
        ID_MENU_COPY_BOOKMARKS_HTML => {
            let html = export_bookmarks_html(&state.bookmarks);
            if !state.bookmarks.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &html);
            }
        }
        ID_MENU_COPY_BOOKMARKS => {
            let text = export_bookmarks_text(&state.bookmarks);
            if !text.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &text);
            }
        }
        ID_MENU_IMPORT_BOOKMARKS => import_bookmarks_from_clipboard(state, hwnd),
        ID_MENU_PASTE_AND_GO => navigate_from_clipboard(state, hwnd, false),
        ID_MENU_SEARCH_CLIPBOARD_NEW_TAB => navigate_from_clipboard(state, hwnd, true),
        ID_MENU_COPY_HISTORY_HTML => {
            let html = export_bookmarks_html(&state.persistent_history);
            if !state.persistent_history.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &html);
            }
        }
        ID_MENU_COPY_OPEN_TABS_HTML => copy_open_tabs_html(state, hwnd),
        ID_MENU_COPY_OPEN_TABS_MARKDOWN => copy_open_tabs_markdown(state, hwnd),
        ID_MENU_COPY_OPEN_TABS_CSV => {
            let entries = state
                .tabs
                .iter()
                .filter_map(|(id, _)| {
                    let page = state.browser.tabs().tab(*id)?.page()?;
                    Some((
                        page.title().unwrap_or_else(|| "(sans titre)".to_owned()),
                        page.url()?.to_string(),
                    ))
                })
                .collect::<Vec<_>>();
            let csv = export_open_tabs_csv(&entries);
            if !csv.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &csv);
            }
        }
        ID_MENU_OPEN_CLIPBOARD_LINKS => open_links_from_clipboard(state, hwnd),
        ID_MENU_COPY_HISTORY => {
            let text = export_bookmarks_text(&state.persistent_history);
            if !text.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &text);
            }
        }
        ID_MENU_COPY_OPEN_TABS => {
            let entries = state
                .tabs
                .iter()
                .filter_map(|(id, _)| {
                    let tab = state.browser.tabs().tab(*id)?;
                    let page = tab.page()?;
                    Some((
                        page.title().unwrap_or_else(|| "(sans titre)".to_owned()),
                        page.url()?.to_string(),
                    ))
                })
                .collect::<Vec<_>>();
            let text = export_open_tabs_text(&entries);
            if !text.is_empty() {
                let _ = copy_text_to_clipboard(hwnd, &text);
            }
        }
        ID_MENU_UA_ARCHIA | ID_MENU_UA_CHROMIUM | ID_MENU_UA_FIREFOX => {
            state.settings.user_agent = match selected {
                ID_MENU_UA_ARCHIA => UserAgentProfile::Archia,
                ID_MENU_UA_CHROMIUM => UserAgentProfile::ChromiumCompatible,
                ID_MENU_UA_FIREFOX => UserAgentProfile::FirefoxCompatible,
                _ => unreachable!(),
            };
            let _ = state.settings.save(&state.settings_path);
        }
        ID_MENU_TOGGLE_SESSION => {
            state.settings.restore_session = !state.settings.restore_session;
            if !state.settings.restore_session {
                let _ = std::fs::remove_file(&state.session_path);
            }
            let _ = state.settings.save(&state.settings_path);
        }
        ID_MENU_OPEN_ALL_BOOKMARKS => open_all_bookmarks(state, hwnd),
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
            if let Some(url) = history_entries
                .iter()
                .rev()
                .nth(index)
                .map(|entry| entry.url.clone())
            {
                navigate(state, &url);
            }
        }
        history_id
            if (ID_MENU_HISTORY_NEW_TAB_BASE
                ..ID_MENU_HISTORY_NEW_TAB_BASE + MAX_MENU_HISTORY_ITEMS as u32)
                .contains(&history_id) =>
        {
            let index = (history_id - ID_MENU_HISTORY_NEW_TAB_BASE) as usize;
            if let Some(url) = history_entries
                .iter()
                .rev()
                .nth(index)
                .map(|entry| entry.url.clone())
            {
                open_link_in_new_tab(state, hwnd, &url);
            }
        }
        bookmark_id
            if (ID_MENU_BOOKMARK_BASE..ID_MENU_BOOKMARK_BASE + MAX_MENU_BOOKMARK_ITEMS as u32)
                .contains(&bookmark_id) =>
        {
            let index = (bookmark_id - ID_MENU_BOOKMARK_BASE) as usize;
            if let Some(bookmark) = state.bookmarks.get(index) {
                let url = bookmark.url.clone();
                navigate(state, &url);
            }
        }
        bookmark_id
            if (ID_MENU_BOOKMARK_NEW_TAB_BASE
                ..ID_MENU_BOOKMARK_NEW_TAB_BASE + MAX_MENU_BOOKMARK_ITEMS as u32)
                .contains(&bookmark_id) =>
        {
            let index = (bookmark_id - ID_MENU_BOOKMARK_NEW_TAB_BASE) as usize;
            if let Some(bookmark) = state.bookmarks.get(index) {
                let url = bookmark.url.clone();
                open_link_in_new_tab(state, hwnd, &url);
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
    // Cadre technique : bande d'onglets sombre, onglet actif fondu dans la barre
    // d'outils avec un filet d'accent, onglets numérotés, angles vifs.
    let strip = chrome_color(state, |colors| &colors.toolbar_background, 0x000F0D0C);
    let window = chrome_color(state, |colors| &colors.window_background, 0x00221D1B);
    let active_tab = chrome_color(state, |colors| &colors.tab_active_background, 0x00221D1B);
    let inactive_tab = chrome_color(state, |colors| &colors.tab_inactive_background, 0x00302926);
    let accent = chrome_color(state, |colors| &colors.accent, 0x004EF2C6);
    let text = chrome_color(state, |colors| &colors.text, 0x00F4F2F1);
    let muted = chrome_color(state, |colors| &colors.muted_text, 0x00B5ABA7);
    let address = chrome_color(state, |colors| &colors.address_background, 0x000F0D0C);
    let border = chrome_color(state, |colors| &colors.border, 0x00423835);

    paint_fill(hdc, 0, 0, width, 88, strip);
    paint_fill(hdc, 0, 39, width, 90, window);
    paint_fill(hdc, 0, 86, width, 89, border);

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
        if selected {
            // Fondu avec la barre d'outils (même couleur) jusqu'à y = 39.
            paint_fill(hdc, left, 7, right, 39, active_tab);
            paint_fill(hdc, left, 7, right, 9, accent);
        } else {
            paint_fill(hdc, left, 11, right, 36, inactive_tab);
        }
        let title = state
            .browser
            .tabs()
            .tab(*id)
            .and_then(|tab| tab.page())
            .and_then(|page| page.title())
            .unwrap_or_else(|| "Nouvel onglet".to_owned());
        let label = title.chars().take(22).collect::<String>();
        let number = format!("{:02}", index + 1);
        paint_label(
            hdc,
            &number,
            left + 10,
            9,
            left + 30,
            36,
            if selected { accent } else { muted },
            0,
        );
        paint_label(
            hdc,
            &label,
            left + 32,
            9,
            right - if state.tabs.len() > 1 { 34 } else { 10 },
            36,
            if selected { text } else { muted },
            0,
        );
        if state.tabs.len() > 1 {
            paint_label(hdc, "×", right - 30, 9, right - 6, 36, muted, DT_CENTER);
        }
    }
    paint_round_fill(hdc, width - 108, 7, width - 62, 36, inactive_tab, 3);
    paint_label(hdc, "⋯", width - 108, 7, width - 62, 36, text, DT_CENTER);
    paint_round_fill(hdc, width - 58, 7, width - 10, 36, inactive_tab, 3);
    paint_label(hdc, "+", width - 58, 7, width - 10, 36, text, DT_CENTER);

    let can_go_back = state.browser.history().can_go_back();
    let can_go_forward = state.browser.history().can_go_forward();
    for (label, left, right, enabled) in [
        ("‹", 10, 44, can_go_back),
        ("›", 48, 82, can_go_forward),
        ("⟳", 86, 122, true),
    ] {
        paint_round_fill(
            hdc,
            left,
            48,
            right,
            80,
            if enabled { inactive_tab } else { window },
            3,
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

    // Champ d'adresse : liseré 1 px puis fond.
    let address_right = (width - 170).max(240);
    paint_round_fill(hdc, 132, 48, address_right, 80, border, 3);
    paint_round_fill(hdc, 133, 49, address_right - 1, 79, address, 3);

    let current_url = state.browser.current_url().map(ToString::to_string);
    let bookmarked = current_url
        .as_ref()
        .is_some_and(|url| state.bookmarks.iter().any(|bookmark| &bookmark.url == url));
    let security_label = security_badge_label(state.browser.current_url());
    let security_color = if security_label == "HTTPS" {
        accent
    } else {
        muted
    };
    paint_round_fill(hdc, 140, 52, 186, 76, inactive_tab, 3);
    paint_label(
        hdc,
        security_label,
        140,
        52,
        186,
        76,
        security_color,
        DT_CENTER,
    );
    paint_round_fill(hdc, width - 166, 48, width - 132, 80, inactive_tab, 3);
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
    paint_round_fill(hdc, width - 122, 48, width - 10, 80, accent, 3);
    paint_label(
        hdc,
        "Aller",
        width - 122,
        48,
        width - 10,
        80,
        DARK_ON_ACCENT,
        DT_CENTER,
    );

    // Barre de favoris, de la même couleur que la barre d'outils.
    paint_fill(hdc, 0, 90, width, 119, window);
    paint_fill(hdc, 0, 118, width, 119, border);
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
            paint_round_fill(hdc, left, 93, left + item_width, 115, inactive_tab, 3);
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
    paint_round_fill(
        hdc,
        left,
        150,
        left + content_width,
        height.min(690),
        card,
        22,
    );
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
    paint_label(
        hdc,
        "Rechercher sur le Web ou saisir une adresse",
        left + 30,
        344,
        left + content_width - 30,
        370,
        muted,
        DT_CENTER,
    );

    let gap = 16;
    let card_width = ((content_width - 80 - gap * 2) / 3).max(120);
    let card_top = 470;
    let quick_links = if state.bookmarks.is_empty() {
        &state.persistent_history
    } else {
        &state.bookmarks
    };
    if quick_links.is_empty() {
        for (index, (title, detail)) in [
            ("Navigation", "Une interface simple et directe"),
            ("Indépendance", "Un moteur développé par Archia"),
            ("Confidentialité", "Pas de moteur tiers embarqué"),
        ]
        .iter()
        .enumerate()
        {
            let x = left + 40 + index as i32 * (card_width + gap);
            paint_round_fill(hdc, x, card_top, x + card_width, card_top + 112, card, 14);
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
    } else {
        paint_label(
            hdc,
            if state.bookmarks.is_empty() {
                "Récemment consultés"
            } else {
                "Accès rapides"
            },
            left + 40,
            430,
            left + content_width - 40,
            462,
            text,
            0,
        );
        for (index, bookmark) in quick_links.iter().take(6).enumerate() {
            let column = index % 3;
            let row = index / 3;
            let x = left + 40 + column as i32 * (card_width + gap);
            let y = card_top + row as i32 * 82;
            paint_round_fill(hdc, x, y, x + card_width, y + 68, card, 14);
            paint_round_fill(hdc, x + 12, y + 13, x + 48, y + 49, accent, 10);
            let initial = bookmark
                .title
                .chars()
                .find(|character| character.is_alphanumeric())
                .map(|character| character.to_uppercase().to_string())
                .unwrap_or_else(|| "★".to_owned());
            paint_label(
                hdc,
                &initial,
                x + 12,
                y + 13,
                x + 48,
                y + 49,
                background,
                DT_CENTER,
            );
            paint_label(
                hdc,
                &bookmark.title,
                x + 58,
                y + 8,
                x + card_width - 10,
                y + 37,
                text,
                0,
            );
            paint_label(
                hdc,
                &bookmark.url,
                x + 58,
                y + 35,
                x + card_width - 10,
                y + 59,
                muted,
                0,
            );
        }
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
    let address_width = (width - 380).max(40);
    let _ = MoveWindow(
        state.address,
        margin + 180,
        button_y + 2,
        address_width,
        button_h - 4,
        1,
    );
    let address_region = CreateRoundRectRgn(0, 0, address_width, button_h - 4, 6, 6);
    if !address_region.is_null() && SetWindowRgn(state.address, address_region, 1) == 0 {
        let _ = DeleteObject(address_region);
    }
    let _ = MoveWindow(state.go, width - 118, button_y, 108, button_h, 1);
    let home_width = (width - 120).clamp(220, 620);
    let home_left = (width - home_width - 82).max(20) / 2;
    let _ = MoveWindow(state.home_search, home_left, 378, home_width, 42, 1);
    let home_region = CreateRoundRectRgn(0, 0, home_width, 42, 20, 20);
    if !home_region.is_null() && SetWindowRgn(state.home_search, home_region, 1) == 0 {
        let _ = DeleteObject(home_region);
    }
    let _ = MoveWindow(state.home_go, home_left + home_width + 10, 378, 72, 42, 1);
    let go_region = CreateRoundRectRgn(0, 0, 72, 42, 20, 20);
    if !go_region.is_null() && SetWindowRgn(state.home_go, go_region, 1) == 0 {
        let _ = DeleteObject(go_region);
    }
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
            state.home_search =
                create_control(hwnd, instance, "EDIT", "", ES_AUTOHSCROLL, ID_HOME_SEARCH);
            state.home_go = create_control(hwnd, instance, "BUTTON", "Rechercher", 0, ID_HOME_GO);
            state.address_brush = CreateSolidBrush(chrome_color(
                state,
                |colors| &colors.address_background,
                0x00F5F3F1,
            ));
            let address_face = wide("Consolas");
            state.address_font = CreateFontW(
                -13,
                0,
                0,
                0,
                500,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                address_face.as_ptr(),
            );
            if !state.address_font.is_null() {
                let _ = SendMessageW(state.address, 0x0030, state.address_font as usize, 1);
            }
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
            let address_hint = wide("Rechercher ou saisir une adresse");
            let home_hint = wide("Rechercher sur le Web");
            let _ = SendMessageW(state.address, 0x1501, 0, address_hint.as_ptr() as Lparam); // EM_SETCUEBANNER
            let _ = SendMessageW(state.home_search, 0x1501, 0, home_hint.as_ptr() as Lparam);
            let _ = ShowWindow(state.page, SW_HIDE);
            let _ = SendMessageW(state.page, 0x0030, 1, 0);
            refresh_page(state);
            let restore = state.startup_session.clone();
            if let Some(first_url) = restore.first() {
                navigate(state, first_url);
                for url in restore.iter().skip(1) {
                    create_new_tab(state, hwnd);
                    navigate(state, url);
                }
            }
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
                        if let Some((id, _)) = state.tabs.get(index).copied() {
                            let tab_left = 10 + index as i32 * tab_width;
                            let close_clicked =
                                state.tabs.len() > 1 && client_x >= tab_left + tab_width - 36;
                            if close_clicked {
                                close_tab_at(state, index);
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
                        record_history(state);
                        save_session(state);
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
                        record_history(state);
                        save_session(state);
                        refresh_page(state);
                    }
                } else if (90..=118).contains(&client_y) && client_x >= 12 {
                    let index = ((client_x - 12) / 162) as usize;
                    if let Some(bookmark) = state.bookmarks.get(index) {
                        let url = bookmark.url.clone();
                        navigate(state, &url);
                    }
                } else if state.browser.current_page().is_none() && (470..=620).contains(&client_y)
                {
                    let content_width = width.min(980);
                    let left = (width - content_width) / 2;
                    let card_width = ((content_width - 80 - 32) / 3).max(120);
                    let column = ((client_x - left - 40) / (card_width + 16)).max(0) as usize;
                    let row = ((client_y - 470) / 82).max(0) as usize;
                    if client_x >= left + 40 && column < 3 {
                        let quick_links = if state.bookmarks.is_empty() {
                            &state.persistent_history
                        } else {
                            &state.bookmarks
                        };
                        if let Some(bookmark) = quick_links.get(row * 3 + column) {
                            let url = bookmark.url.clone();
                            navigate(state, &url);
                        }
                    }
                } else if GetKeyState(VK_CONTROL) < 0 {
                    if let Some(url) = link_url_at(state, client_x, client_y) {
                        open_link_in_new_tab(state, hwnd, &url);
                    } else {
                        activate_link_at(state, hwnd, client_x, client_y);
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
        WM_RBUTTONDOWN => {
            if let Some(state) = app_state(hwnd) {
                let client_x = (l_param as u32 & 0xffff) as u16 as i16 as i32;
                let client_y = ((l_param as u32 >> 16) & 0xffff) as u16 as i16 as i32;
                if (90..=118).contains(&client_y) && client_x >= 12 {
                    let index = ((client_x - 12) / 162) as usize;
                    show_bookmark_context_menu(state, hwnd, index);
                } else if client_y > 120 {
                    if let Some(url) = link_url_at(state, client_x, client_y) {
                        show_link_context_menu(state, hwnd, url);
                    } else {
                        show_page_context_menu(state, hwnd);
                    }
                } else if (0..=42).contains(&client_y) && client_x >= 10 {
                    let mut client = Rect {
                        left: 0,
                        top: 0,
                        right: 1180,
                        bottom: 820,
                    };
                    if GetClientRect(hwnd, &mut client) != 0 {
                        let width = client.right.saturating_sub(client.left);
                        if client_x < width - 108 {
                            let tab_width = ((width - 134).max(0) / state.tabs.len().max(1) as i32)
                                .clamp(86, 220);
                            let index = ((client_x - 10) / tab_width.max(1)) as usize;
                            show_tab_context_menu(state, hwnd, index);
                        }
                    }
                }
            }
            0
        }
        WM_MBUTTONDOWN => {
            if let Some(state) = app_state(hwnd) {
                let client_x = (l_param as u32 & 0xffff) as u16 as i16 as i32;
                let client_y = ((l_param as u32 >> 16) & 0xffff) as u16 as i16 as i32;
                let mut client = Rect {
                    left: 0,
                    top: 0,
                    right: 1180,
                    bottom: 820,
                };
                if GetClientRect(hwnd, &mut client) != 0
                    && (0..=42).contains(&client_y)
                    && client_x >= 10
                    && client_x < client.right - 108
                {
                    let width = client.right.saturating_sub(client.left);
                    let tab_width =
                        ((width - 134).max(0) / state.tabs.len().max(1) as i32).clamp(86, 220);
                    let index = ((client_x - 10) / tab_width.max(1)) as usize;
                    close_tab_at(state, index);
                } else if client_y > 120 {
                    if let Some(url) = link_url_at(state, client_x, client_y) {
                        open_link_in_new_tab(state, hwnd, &url);
                    }
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
                            record_history(state);
                            save_session(state);
                            refresh_page(state);
                        }
                    }
                    ID_FORWARD => {
                        if state
                            .browser
                            .go_forward(&state.loader, current_viewport(state))
                            .is_ok()
                        {
                            record_history(state);
                            save_session(state);
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
                save_session(state);
                save_persistent_history(state);
                if !state.address_brush.is_null() {
                    let _ = DeleteObject(state.address_brush);
                    state.address_brush = ptr::null_mut();
                }
                if !state.address_font.is_null() {
                    let _ = DeleteObject(state.address_font);
                    state.address_font = ptr::null_mut();
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
            address_font: ptr::null_mut(),
            address_proc: 0,
            home_search_proc: 0,
            surface: None,
            surface_dirty: true,
            scroll_y: 0,
            dib_pixels: Vec::new(),
            tabs: Vec::new(),
            bookmarks: load_bookmarks(&bookmark_storage_path()),
            bookmarks_path: bookmark_storage_path(),
            history_path: history_storage_path(),
            session_path: session_storage_path(),
            persistent_history: load_persistent_history(&history_storage_path()),
            startup_session: {
                let settings = BrowserSettings::load(&settings_storage_path());
                if settings.restore_session {
                    load_session(&session_storage_path())
                } else {
                    Vec::new()
                }
            },
            settings: BrowserSettings::load(&settings_storage_path()),
            settings_path: settings_storage_path(),
            closed_tabs: Vec::new(),
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
    fn close_same_site_keeps_selected_tab_and_ignores_other_hosts() {
        let hosts = vec![
            Some("example.com".into()),
            Some("other.test".into()),
            Some("EXAMPLE.COM".into()),
            None,
            Some("example.com".into()),
        ];
        assert_eq!(tab_indices_for_same_site(&hosts, 2), vec![4, 0]);
        assert!(tab_indices_for_same_site(&hosts, 3).is_empty());
        assert!(tab_indices_for_same_site(&hosts, 8).is_empty());
    }

    #[test]
    fn copying_same_site_urls_keeps_tab_order_and_ignores_other_hosts() {
        let hosts = vec![
            Some("example.com".into()),
            Some("other.test".into()),
            Some("EXAMPLE.COM".into()),
            None,
            Some("example.com".into()),
        ];
        let urls = vec![
            Some("https://example.com/first".into()),
            Some("https://other.test/".into()),
            Some("https://example.com/second".into()),
            None,
            Some("https://example.com/third".into()),
        ];
        assert_eq!(
            urls_for_same_site(&hosts, &urls, 2),
            vec![
                "https://example.com/first",
                "https://example.com/second",
                "https://example.com/third",
            ]
        );
        assert!(urls_for_same_site(&hosts, &urls, 3).is_empty());
        assert!(urls_for_same_site(&hosts, &urls, 9).is_empty());
    }

    #[test]
    fn close_duplicate_urls_keeps_selected_tab_and_preserves_distinct_pages() {
        let urls = vec![
            Some("https://example.com/page".into()),
            Some("https://example.com/other".into()),
            Some("https://example.com/page".into()),
            None,
            Some("https://example.com/page".into()),
        ];
        assert_eq!(tab_indices_for_duplicate_urls(&urls, 2), vec![4, 0]);
        assert!(tab_indices_for_duplicate_urls(&urls, 3).is_empty());
        assert!(tab_indices_for_duplicate_urls(&urls, 8).is_empty());
    }

    #[test]
    fn clearing_recently_closed_tabs_returns_count_and_empties_list() {
        let mut closed = vec![Some("https://one.example/".into()), None];
        assert_eq!(clear_closed_tabs(&mut closed), 2);
        assert!(closed.is_empty());
        assert_eq!(clear_closed_tabs(&mut closed), 0);
    }

    #[test]
    fn recently_closed_tab_indices_map_newest_first_and_reject_out_of_range() {
        assert_eq!(closed_tab_storage_index(3, 0), Some(2));
        assert_eq!(closed_tab_storage_index(3, 1), Some(1));
        assert_eq!(closed_tab_storage_index(3, 2), Some(0));
        assert_eq!(closed_tab_storage_index(3, 3), None);
        assert_eq!(closed_tab_storage_index(0, 0), None);
    }

    #[test]
    fn closed_tabs_markdown_export_filters_invalid_urls_and_keeps_order() {
        let closed = vec![
            Some("https://example.com/first".to_owned()),
            None,
            Some("javascript:alert(1)".to_owned()),
            Some("https://example.org/second".to_owned()),
        ];
        let bookmarks = closed_tabs_as_bookmarks(&closed);
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].url, "https://example.com/first");
        assert_eq!(bookmarks[1].url, "https://example.org/second");
        assert!(export_bookmarks_markdown(&bookmarks).contains("example.com/first"));
    }

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
    fn alternate_search_providers_encode_queries() {
        assert_eq!(
            parse_address_with_provider("rust browser", SearchProvider::Bing)
                .unwrap()
                .to_string(),
            "https://www.bing.com/search?q=rust%20browser"
        );
        assert_eq!(
            parse_address_with_provider("rust browser", SearchProvider::DuckDuckGo)
                .unwrap()
                .to_string(),
            "https://duckduckgo.com/?q=rust%20browser"
        );
    }

    #[test]
    fn alternate_search_provider_does_not_rewrite_explicit_urls() {
        assert_eq!(
            parse_address_with_provider("https://example.org/path", SearchProvider::Bing)
                .unwrap()
                .to_string(),
            "https://example.org/path"
        );
    }

    #[test]
    fn bookmarks_sort_by_title_without_losing_urls() {
        let mut bookmarks = vec![
            Bookmark {
                title: "Zulu".into(),
                url: "https://z.example/".into(),
            },
            Bookmark {
                title: "alpha".into(),
                url: "https://a.example/".into(),
            },
            Bookmark {
                title: "Beta".into(),
                url: "https://b.example/".into(),
            },
        ];
        sort_bookmarks_by_title(&mut bookmarks);
        assert_eq!(
            bookmarks
                .iter()
                .map(|bookmark| bookmark.title.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "Beta", "Zulu"]
        );
        assert_eq!(bookmarks[0].url, "https://a.example/");
    }

    #[test]
    fn bookmarks_sort_by_site_then_title() {
        let mut bookmarks = vec![
            Bookmark {
                title: "Z page".into(),
                url: "https://z.example/".into(),
            },
            Bookmark {
                title: "B page".into(),
                url: "https://a.example/b".into(),
            },
            Bookmark {
                title: "A page".into(),
                url: "https://a.example/a".into(),
            },
        ];
        sort_bookmarks_by_domain(&mut bookmarks);
        assert_eq!(
            bookmarks
                .iter()
                .map(|bookmark| bookmark.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A page", "B page", "Z page"]
        );
    }

    #[test]
    fn duplicate_bookmarks_keep_the_first_entry_and_export_as_tsv() {
        let mut bookmarks = vec![
            Bookmark {
                title: "First title".into(),
                url: "https://example.org/".into(),
            },
            Bookmark {
                title: "Duplicate".into(),
                url: "https://example.org/".into(),
            },
            Bookmark {
                title: "Other".into(),
                url: "https://other.example/".into(),
            },
        ];
        deduplicate_bookmarks(&mut bookmarks);
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].title, "First title");
        assert_eq!(
            export_bookmarks_text(&bookmarks),
            "First title\thttps://example.org/\nOther\thttps://other.example/"
        );
    }

    #[test]
    fn history_deduplication_keeps_the_latest_visit_and_order() {
        let mut history = vec![
            Bookmark {
                title: "Old title".into(),
                url: "https://same.example/".into(),
            },
            Bookmark {
                title: "Middle".into(),
                url: "https://middle.example/".into(),
            },
            Bookmark {
                title: "Latest title".into(),
                url: "https://same.example/".into(),
            },
        ];
        deduplicate_history_keep_latest(&mut history);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].title, "Middle");
        assert_eq!(history[1].title, "Latest title");
    }

    #[test]
    fn settings_round_trip_and_reject_unsafe_homepages() {
        let path = std::env::temp_dir().join(format!(
            "archia-browser-settings-{}.ini",
            std::process::id()
        ));
        let settings = BrowserSettings {
            search_provider: SearchProvider::DuckDuckGo,
            homepage: "https://example.org/start".into(),
            restore_session: false,
            user_agent: UserAgentProfile::FirefoxCompatible,
        };
        settings.save(&path).unwrap();
        assert_eq!(BrowserSettings::load(&path), settings);
        std::fs::write(
            &path,
            "search_provider=unknown\nhomepage=file:///etc/passwd\nrestore_session=maybe\nuser_agent=unknown\n",
        )
        .unwrap();
        assert_eq!(BrowserSettings::load(&path), BrowserSettings::default());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn page_text_truncation_respects_utf8_boundaries() {
        let mut text = "Aé🙂Z".to_owned();
        truncate_utf8(&mut text, 4);
        assert_eq!(text, "Aé");
        let mut short = "Archia".to_owned();
        truncate_utf8(&mut short, 20);
        assert_eq!(short, "Archia");
    }

    #[test]
    fn user_agent_profiles_are_distinct_and_persisted() {
        assert!(UserAgentProfile::Archia
            .header("1.2")
            .contains("ArchiaBrowser/1.2"));
        assert!(UserAgentProfile::ChromiumCompatible
            .header("1.2")
            .contains("Chrome/131.0.0.0"));
        assert!(UserAgentProfile::FirefoxCompatible
            .header("1.2")
            .contains("Firefox/128.0"));

        let path = std::env::temp_dir().join(format!(
            "archia-browser-user-agent-{}.ini",
            std::process::id()
        ));
        let settings = BrowserSettings {
            user_agent: UserAgentProfile::ChromiumCompatible,
            ..BrowserSettings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(BrowserSettings::load(&path), settings);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn download_names_are_safe_and_collisions_are_avoided() {
        assert_eq!(safe_download_filename("../CON"), ".._CON");
        assert_eq!(safe_download_filename("report?.zip"), "report_.zip");
        let directory =
            std::env::temp_dir().join(format!("archia-download-names-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let existing = directory.join("report.zip");
        std::fs::write(&existing, b"existing").unwrap();
        assert_eq!(
            unique_download_destination(&directory, "report.zip"),
            directory.join("report (1).zip")
        );
        let _ = std::fs::remove_file(existing);
        let _ = std::fs::remove_dir(directory);
    }

    #[test]
    fn dotted_hosts_still_open_directly() {
        assert_eq!(
            parse_address("example.org/path").unwrap().to_string(),
            "https://example.org/path"
        );
    }

    #[test]
    fn localhost_with_port_opens_as_a_host() {
        assert_eq!(
            parse_address("localhost:8080").unwrap().to_string(),
            "https://localhost:8080/"
        );
    }

    #[test]
    fn ipv4_and_bracketed_ipv6_hosts_open_directly() {
        assert_eq!(
            parse_address("192.168.1.20:3000").unwrap().to_string(),
            "https://192.168.1.20:3000/"
        );
        assert_eq!(
            parse_address("[::1]:8080").unwrap().to_string(),
            "https://[::1]:8080/"
        );
    }

    #[test]
    fn bookmark_import_accepts_both_tsv_column_orders_and_filters_unsafe_urls() {
        let imported = parse_imported_bookmarks(
            "Docs\thttps://docs.example/path\nhttps://archia.example/\tArchia\nBad\tfile:///etc/passwd\nDuplicate\thttps://docs.example/path\nNo separator",
        );
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].title, "Docs");
        assert_eq!(imported[0].url, "https://docs.example/path");
        assert_eq!(imported[1].title, "Archia");
        assert_eq!(imported[1].url, "https://archia.example/");

        let url_only = parse_imported_bookmarks(
            "https://one.example/path\nmailto:test@example.com\nhttps://two.example/",
        );
        assert_eq!(url_only.len(), 2);
        assert_eq!(url_only[0].title, "one.example");
        assert_eq!(url_only[1].url, "https://two.example/");
    }

    #[test]
    fn markdown_bookmark_import_and_export_round_trip() {
        let bookmarks = vec![
            Bookmark {
                title: "Docs [draft]".to_owned(),
                url: "https://docs.example/path_(one)".to_owned(),
            },
            Bookmark {
                title: "Archia".to_owned(),
                url: "https://archia.example/".to_owned(),
            },
        ];
        let markdown = export_bookmarks_markdown(&bookmarks);
        assert!(markdown.contains("[Docs \\[draft\\]](<https://docs.example/path_(one)>)"));
        assert_eq!(parse_imported_bookmarks(&markdown), bookmarks);
        assert!(parse_imported_bookmarks("[unsafe](javascript:alert(1))").is_empty());
    }

    #[test]
    fn netscape_bookmark_import_and_html_export_round_trip() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
            <DL><p>
            <DT><A HREF="https://example.org/docs?a=1&amp;b=2">Docs &amp; Guides</A>
            <DT><A HREF='http://archia.example/'>Archia</A>
            <DT><A HREF="javascript:alert(1)">Unsafe</A>
            <DT><A HREF="https://example.org/docs?a=1&amp;b=2">Duplicate</A>
            </DL>"#;
        let bookmarks = parse_netscape_bookmarks(html);
        assert_eq!(bookmarks.len(), 2);
        assert_eq!(bookmarks[0].title, "Docs & Guides");
        assert_eq!(bookmarks[0].url, "https://example.org/docs?a=1&b=2");
        assert_eq!(bookmarks[1].title, "Archia");

        let exported = export_bookmarks_html(&bookmarks);
        assert!(exported.contains("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
        assert!(exported.contains("Docs &amp; Guides"));
        assert!(exported.contains("a=1&amp;b=2"));
        assert_eq!(parse_imported_bookmarks(&exported), bookmarks);
    }

    #[test]
    fn clipboard_navigation_accepts_urls_and_searches_plain_text() {
        assert_eq!(
            clipboard_navigation_url("  https://example.org/docs  ", SearchProvider::Google)
                .unwrap()
                .to_string(),
            "https://example.org/docs"
        );
        assert_eq!(
            clipboard_navigation_url("archia browser", SearchProvider::DuckDuckGo)
                .unwrap()
                .to_string(),
            "https://duckduckgo.com/?q=archia%20browser"
        );
    }

    #[test]
    fn clipboard_navigation_rejects_unsafe_empty_and_oversized_values() {
        assert!(clipboard_navigation_url("", SearchProvider::Google).is_none());
        assert!(clipboard_navigation_url("javascript:alert(1)", SearchProvider::Google).is_none());
        assert!(clipboard_navigation_url("javascript:", SearchProvider::Google).is_none());
        assert!(clipboard_navigation_url("http:example.org", SearchProvider::Google).is_none());
        assert!(
            clipboard_navigation_url("mailto:test@example.com", SearchProvider::Google).is_none()
        );
        assert!(
            clipboard_navigation_url("ftp://example.org/file", SearchProvider::Google).is_none()
        );
        assert!(clipboard_navigation_url("file:///etc/passwd", SearchProvider::Google).is_none());
        assert!(clipboard_navigation_url(&"x".repeat(4_097), SearchProvider::Google).is_none());
        assert!(clipboard_navigation_url("hello\nworld", SearchProvider::Google).is_none());
    }

    #[test]
    fn clipboard_navigation_keeps_localhost_ports_and_http_schemes() {
        assert_eq!(
            clipboard_navigation_url("localhost:8080", SearchProvider::Google)
                .unwrap()
                .to_string(),
            "https://localhost:8080/"
        );
        assert_eq!(
            clipboard_navigation_url("https://example.org/path", SearchProvider::Google)
                .unwrap()
                .to_string(),
            "https://example.org/path"
        );
    }

    #[test]
    fn markdown_link_escapes_label_delimiters_and_falls_back_to_url() {
        assert_eq!(
            markdown_link("Docs [draft]", "https://example.org/docs"),
            "[Docs \\[draft\\]](<https://example.org/docs>)"
        );
        assert_eq!(
            markdown_link("", "https://example.org/"),
            "[https://example.org/](<https://example.org/>)"
        );
    }

    #[test]
    fn open_tabs_csv_export_escapes_quotes_preserves_order_and_filters_unsafe_urls() {
        let entries = vec![
            (
                "Docs, \"draft\"".to_owned(),
                "https://docs.example/path".to_owned(),
            ),
            ("Unsafe".to_owned(), "javascript:alert(1)".to_owned()),
            ("Second".to_owned(), "https://example.org/".to_owned()),
        ];
        assert_eq!(
            export_open_tabs_csv(&entries),
            "title,url\\r\\n\\"Docs, \\"\\"draft\\"\\",\\"https://docs.example/path\\"\\r\\n\\"Second\\",\\"https://example.org/\\""
        );
        assert!(export_open_tabs_csv(&[]).is_empty());
        assert!(export_open_tabs_csv(&[(
            "Unsafe".to_owned(),
            "javascript:alert(1)".to_owned()
        )])
        .is_empty());
    }

    #[test]
    fn open_tabs_markdown_export_preserves_order_and_filters_unsafe_urls() {
        let entries = vec![
            (
                "Docs [draft]".to_owned(),
                "https://docs.example/".to_owned(),
            ),
            ("Unsafe".to_owned(), "javascript:alert(1)".to_owned()),
            ("Archia".to_owned(), "https://archia.example/".to_owned()),
        ];
        assert_eq!(
            export_open_tabs_markdown(&entries),
            "- [Docs \\[draft\\]](<https://docs.example/>)\n- [Archia](<https://archia.example/>)"
        );
        assert!(export_open_tabs_markdown(&[]).is_empty());
    }

    #[test]
    fn open_tabs_html_export_is_netscape_compatible_and_escapes_titles() {
        let entries = vec![
            (
                "A & B <Docs>".to_owned(),
                "https://example.org/docs".to_owned(),
            ),
            ("Unsafe".to_owned(), "javascript:alert(1)".to_owned()),
        ];
        let html = export_open_tabs_html(&entries);
        assert!(html.contains("A &amp; B &lt;Docs&gt;"));
        assert!(html.contains("HREF=\"https://example.org/docs\""));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn open_tabs_export_is_tab_separated_and_preserves_order() {
        let entries = vec![
            ("Archia".to_owned(), "https://archia.example/".to_owned()),
            ("Docs".to_owned(), "https://docs.example/".to_owned()),
        ];
        assert_eq!(
            export_open_tabs_text(&entries),
            "Archia\thttps://archia.example/\nDocs\thttps://docs.example/"
        );
        assert!(export_open_tabs_text(&[]).is_empty());
    }

    #[test]
    fn close_other_tab_indices_keep_only_the_active_tab() {
        assert_eq!(tab_indices_to_close_except(2, 5), vec![4, 3, 1, 0]);
        assert!(tab_indices_to_close_except(0, 0).is_empty());
        assert!(tab_indices_to_close_except(8, 5).is_empty());
    }

    #[test]
    fn close_tab_ranges_exclude_active_tab_and_close_in_reverse_order() {
        assert_eq!(tab_indices_to_close(2, 5, true), vec![1, 0]);
        assert_eq!(tab_indices_to_close(2, 5, false), vec![4, 3]);
        assert!(tab_indices_to_close(0, 1, true).is_empty());
        assert!(tab_indices_to_close(4, 5, false).is_empty());
        assert!(tab_indices_to_close(5, 5, false).is_empty());
    }

    #[test]
    fn security_badge_distinguishes_https_http_and_empty_tabs() {
        let https = parse_address("https://example.org").unwrap();
        let http = parse_address("http://example.org").unwrap();
        assert_eq!(security_badge_label(Some(&https)), "HTTPS");
        assert_eq!(security_badge_label(Some(&http)), "HTTP");
        assert_eq!(security_badge_label(None), "WEB");
    }
}
