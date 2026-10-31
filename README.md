# Archia Browser

> A homegrown web browser for Windows first, then native integration with ArchiaOS.

- 🖥️ **ArchiaOS:** https://github.com/Likounsee/ArchiaOS
- 💾 **OpenFS:** https://github.com/Likounsee/OpenFS

![Status](https://img.shields.io/badge/status-active%20development-orange)
![Targets](https://img.shields.io/badge/targets-Windows%20first%20%7C%20ArchiaOS%20next-blue)
![Engine](https://img.shields.io/badge/engine-homegrown-green)

Archia Browser is built around its own browser engine. It does not embed Chromium/Blink, Gecko, WebKit, Servo or another third-party browser engine.

## Platform order

Development has a strict order:

1. **Windows 11 first:** deliver an actually usable desktop browser on Windows, with a native window, address bar, rendered pages and working navigation.
2. **ArchiaOS afterwards:** port the platform-facing layer and integrate the same browser core when ArchiaOS has enough system, windowing and graphics support.

The shared HTML, DOM, CSS, layout, rendering, URL, networking and navigation code should remain platform-independent wherever practical. Windows-specific windowing/input/display code must stay behind a platform boundary so the later ArchiaOS backend does not require a second browser engine.

ArchiaOS is currently at a very early development stage. Its browser port is therefore a later milestone, not a dependency for the Windows release.

## Current application status

The current Windows application now starts in an **experimental native desktop GUI**. It uses Win32 windowing and controls directly; it does not embed WebView2, Edge, Chromium or another browser engine.

On Windows, from the repository root, run:

```powershell
.\scripts\run-windows.ps1 https://example.org
```

Or start with a blank tab:

```powershell
.\scripts\run-windows.ps1
```

The GUI currently provides a native window, a custom-painted browser toolbar and tab strip, address/search fields, Back, Forward and Reload, a new-tab home screen, and a software-rendered page viewport. Bookmarks are saved locally and can be opened from the toolbar, the browser menu or quick-access tiles on the new-tab page. Right-click a bookmark to open it, open it in a new tab or remove it. The browser menu exposes recent in-memory history and built-in interface themes. Tabs support middle-click closing and a context menu for closing, duplicating or closing other tabs. Keyboard shortcuts include Ctrl+L/Ctrl+K (focus address), Ctrl+T (new tab), Ctrl+W (close tab), Ctrl+Shift+T (reopen closed tab), Ctrl+Tab/Ctrl+Shift+Tab (cycle tabs), Ctrl+1…Ctrl+9 (select tab), Ctrl+D (bookmark), Ctrl+H (history menu), Ctrl+R and F5 (reload), and Alt+Left/Right (history navigation). Navigation uses Archia's own HTTP/HTTPS transport, document loader, HTML/DOM and navigation state. The page viewport presents Archia's own software-rendered display list through a native Win32 DIB; no embedded browser engine or WebView is used. Resizing the window recalculates layout from the existing styled DOM instead of refetching or reparsing the page. This remains an early browser UI and renderer: modern CSS coverage, text shaping, image decoding/painting, JavaScript, forms, downloads, persistent browsing history and modern-site compatibility remain incomplete.

On non-Windows systems the executable remains a terminal-based development client. It supports entering addresses and the commands `:links`, `:open NUMERO`, `:back`, `:forward`, `:reload`, `:history`, `:url`, `:title`, `:help` and `:quit`.

Theme manifests and selection are available in the engine, but the current Win32 frontend does not yet apply the theme palette to its controls. On Windows, theme data is stored under `%APPDATA%\\ArchiaBrowser\\themes`.

## Immediate Windows product roadmap

### W0 — Runnable Windows application
- [x] Cargo executable and interactive session.
- [x] HTTP/HTTPS top-level navigation.
- [x] Back, forward and reload.
- [x] Extract and list document links.
- [x] Open a listed link.
- [x] PowerShell launcher.
- [x] Native Windows window and message loop.
- [x] Address bar and navigation controls.
- [x] Display Archia's software-rendered page surface through native Win32 presentation.
- [x] Mouse hit-testing and clickable links.
- [x] Resize handling and viewport re-layout without reparsing the document.
- [ ] Windows packaging and a repeatable install/run workflow.

### W1 — Usable browser essentials
- [x] Multiple tabs and tab strip (create and switch).
- [x] Scrollable rendered page viewport (native vertical scrollbar and mouse-wheel scrolling).
- [x] Local persistent bookmarks and bookmark-bar interaction.\n- [ ] Persistent browsing history.
- [ ] Download handling and download destination controls.
- [ ] Better error pages and loading feedback.
- [ ] Basic settings and user-agent controls.


### W2 — Theme system
- [x] Platform-independent theme manifest and palette model.
- [x] Built-in Archia, Chrome-inspired, Firefox-inspired and Edge-inspired themes.
- [x] Persistent theme selection per user.
- [x] Install and uninstall custom data-only theme manifests through the engine API.
- [x] Native GUI applies built-in theme colors to browser chrome.\n- [ ] Tab placement and sidebar layout follow the selected theme.
- [ ] Theme preview and settings screen.
- [ ] Downloadable theme gallery with source/signature verification and update flow.
- [ ] Export and import complete theme packages, including assets.

Theme packages are data-only. The first manifest format defines browser-chrome colors, layout preset, tab placement and sidebar preference. It does not execute scripts, load native code or alter the page's own CSS. The built-in Chrome/Firefox/Edge options are interface inspirations, not official products or endorsements; their trademarks remain the property of their owners.

### W3 — Web compatibility and safety
- [ ] JavaScript runtime and DOM bindings.
- [ ] Forms and common Web APIs.
- [ ] Real font loading and shaping.
- [ ] Images and more complete CSS/layout.
- [ ] Stronger process isolation and sandboxing before enabling untrusted active content.
- [ ] Compatibility testing against real websites.

### A0 — ArchiaOS port (after Windows)
- [ ] Define the ArchiaOS window/input/surface backend contract.
- [ ] Implement native ArchiaOS window creation and event delivery.
- [ ] Present the shared software-rendered surface.
- [ ] Integrate filesystem, downloads and system URL handling.
- [ ] Package Archia Browser as an ArchiaOS application.

## Engine foundation

The engine is intentionally separated into independent subsystems:

- HTML tokenizer, parser and DOM.
- CSS tokenizer, selectors, cascade and computed styles.
- Block/inline layout and box model.
- Display-list rendering and RGBA software surfaces.
- URL parsing, HTTP/HTTPS transport, redirects, cookies and cache.
- Browser navigation, history and tab state.
- Security policy and shared memory accounting.

These are foundations, not a claim of full browser compatibility. A successful HTTP response or a parsed page does not mean that a modern website will render correctly.

## Engineering rules

- Windows desktop usability is the immediate product goal.
- ArchiaOS porting starts only after the Windows application is genuinely usable.
- Keep the browser engine independent from the platform windowing layer.
- Do not embed Chromium, Electron, Gecko, WebKit or Servo.
- Prefer coherent, tested feature batches over isolated cosmetic changes.
- GitHub Actions is the authoritative build/test signal; never report checks as passed unless their result is visible for the exact commit.
- Keep platform-specific code isolated and retain a non-Windows development path where practical.

## License

License to be defined.
