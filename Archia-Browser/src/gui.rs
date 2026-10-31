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
const COLOR_WINDOW: usize = 5;

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
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
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

fn page_text(state: &AppState) -> String {
    let Some(page) = state.browser.current_page() else {
        return "Bienvenue dans Archia Browser\n\nSaisissez une adresse dans la barre ci-dessus puis cliquez sur Aller.\n\nLe moteur HTML/CSS natif d'Archia Browser est en cours de développement.".to_owned();
    };
    let mut output = page.document.visible_text_content();
    if output.len() > 2 * 1024 * 1024 {
        output.truncate(2 * 1024 * 1024);
        output.push_str("\n\n[Contenu tronqué : limite d'affichage atteinte]");
    }
    output
}

unsafe fn refresh_page(state: &mut AppState) {
    set_control_text(state.page, &page_text(state));
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
    let _ = SendMessageW(state.page, EM_SETLIMITTEXT, 2 * 1024 * 1024, 0);
    let viewport = LayoutViewport::new(1200, 760);
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
        let _ = MoveWindow(*handle, margin + index as i32 * tab_width, 8, tab_width - 4, 30, 1);
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
            let initial_tab = state.browser.active_tab_id().expect("browser starts with one tab");
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
        WM_SIZE => {
            if let Some(state) = app_state(hwnd) {
                let width = (l_param as u32 & 0xffff) as i32;
                let height = ((l_param as u32 >> 16) & 0xffff) as i32;
                layout_controls(state, width, height);
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
                            .go_back(&state.loader, LayoutViewport::new(1200, 760))
                            .is_ok()
                        {
                            refresh_page(state);
                        }
                    }
                    ID_FORWARD => {
                        if state
                            .browser
                            .go_forward(&state.loader, LayoutViewport::new(1200, 760))
                            .is_ok()
                        {
                            refresh_page(state);
                        }
                    }
                    ID_RELOAD => {
                        if state
                            .browser
                            .reload(&state.loader, LayoutViewport::new(1200, 760))
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
                            let mut rect = Rect { left: 0, top: 0, right: 1180, bottom: 820 };
                            let _ = rect;
                            layout_controls(state, 1180, 820);
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
            tabs: Vec::new(),
        });
        let state_ptr = &mut *state as *mut AppState;
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
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
