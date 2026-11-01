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

The current Windows application now starts in an **experimental native desktop GUI**. It uses Win32 windowing and controls directly; it does not embed WebView2, Edge, Chromium or another browser engine. The custom browser chrome includes a distinct HTTPS/HTTP indicator and a monospaced address field; local hostnames, IPv4 addresses and bracketed IPv6 addresses can be entered directly.

On Windows, from the repository root, run:

```powershell
.\scripts\run-windows.ps1 https://example.org
```

Or start with a blank tab:

```powershell
.\scripts\run-windows.ps1
```

The GUI currently provides a native window, a custom-painted browser toolbar and tab strip, address/search fields, Back, Forward and Reload, a new-tab home screen, and a software-rendered page viewport. Browsing history is stored locally (up to 500 recent URLs), exposed from the browser menu and retained between launches; Ctrl+Shift+Delete or the browser menu clears it. Recent pages appear as clickable tiles on the new-tab page when no bookmarks are available. The last session's open HTTP/HTTPS tabs are restored on startup (up to 20 tabs), and session restoration can be disabled from the browser menu. Preferences are stored in `settings.ini`: choose Google, Bing or DuckDuckGo as the address-bar search provider, set the current page as the homepage, reset it, and open the homepage with Ctrl+Home. The browser menu can download the current HTTP/HTTPS page to the user's Downloads directory; the download pipeline follows bounded redirects, blocks HTTPS downgrades, enforces a configurable body-size ceiling and publishes files atomically without overwriting existing files. The reusable download API is also covered by mock-transport tests. The same menu opens the Downloads folder, and Ctrl+J provides a direct keyboard shortcut. A recent-downloads menu lists the 12 most recently modified files in Downloads and can open a selected file, reveal it in File Explorer or copy its full path; temporary in-progress files are excluded. Bookmarks are saved locally and can be opened from the toolbar, the browser menu or quick-access tiles on the new-tab page; the browser menu can open all saved bookmarks in separate tabs. Right-click a bookmark to open it, open it in a new tab or remove it. The browser menu also offers a one-click new-tab action for each of the eight most recent history entries and the first 16 bookmarks; Ctrl+Shift+H opens the eight most recently visited pages in separate tabs, and the browser menu provides the same batch action, Ctrl+Shift+O opens saved bookmarks in separate tabs, and Ctrl+Shift+J opens the recent-downloads menu. The browser menu can also sort bookmarks alphabetically or by site, remove duplicate URLs while keeping the first saved entry, and copy the complete bookmark list as tab-separated text or standard Netscape bookmark HTML; Ctrl+Shift+B copies the HTML export. Bookmark import accepts Netscape HTML exports from common browsers as well as tab-separated text and plain URL lists, while filtering unsafe URL schemes and duplicate entries. It can also copy the browsing history in the same format; duplicate history URLs loaded from older profile files are collapsed while preserving their latest visit. The browser menu can clear the in-memory cookie jar and HTTP response cache for the current session without deleting browsing history or bookmarks. A separate hard-reload action (Ctrl+Shift+R or the browser menu) clears only the HTTP response cache before reloading, preserving cookies and the rest of the session. It also persists an Archia, Chromium-compatible or Firefox-compatible User-Agent profile for subsequent page navigations and downloads; this changes the request header only, not the rendering engine. Right-click the page to copy its title or its visible text; Ctrl+Shift+X copies the visible page text (up to 2 MiB, without splitting UTF-8 characters). Ctrl+Shift+E opens the current HTTP/HTTPS page in the Windows default browser; the same action is available in the page context menu. Right-click the page viewport for page actions: copy the current URL to the Windows clipboard, add/remove the current page from bookmarks, reload or download it; the bookmark action reflects whether the page is already saved. Right-click a rendered HTTP/HTTPS link for link-specific actions: open it in a new tab, copy its target URL or download the target directly. Ctrl+Shift+C copies the current page URL; Ctrl+Shift+K duplicates the current page into a new tab; Alt+Home opens the configured homepage. Ctrl+PageUp and Ctrl+PageDown cycle between tabs. The tab context menu can reload or copy that tab's URL, close tabs to its right, close all other tabs, close other tabs from the same site, or close tabs whose full URL exactly matches the selected tab while keeping the selected tab open, or copy all open URLs from the selected tab's site to the clipboard in tab order. Links open in the current tab with a normal click, and in a new tab with Ctrl+click or middle-click. Tabs support middle-click closing and a context menu for closing, duplicating or closing other tabs. Keyboard shortcuts include F6 (focus and select the address bar), Ctrl+L/Ctrl+K (focus address), Ctrl+T (new tab), Ctrl+W (close tab), Ctrl+Shift+T (reopen closed tab, also available from the browser menu), Ctrl+J (open Downloads folder), Ctrl+Tab/Ctrl+Shift+Tab (cycle tabs), Ctrl+1…Ctrl+9 (select tab), Ctrl+D (bookmark), Ctrl+H (history menu), Ctrl+Home (homepage), Ctrl+R and F5 (reload), Ctrl+Shift+Delete (clear history), and Alt+Left/Right (history navigation). Navigation uses Archia's own HTTP/HTTPS transport, document loader, HTML/DOM and navigation state. The page viewport presents Archia's own software-rendered display list through a native Win32 DIB; no embedded browser engine or WebView is used. Resizing the window recalculates layout from the existing styled DOM instead of refetching or reparsing the page. This remains an early browser UI and renderer: modern CSS coverage, text shaping, image decoding/painting, JavaScript, forms and modern-site compatibility remain incomplete; the initial download workflow is synchronous and does not yet provide a background queue or progress UI.

On non-Windows systems the executable remains a terminal-based development client. It supports entering addresses and the commands `:links`, `:open NUMERO`, `:back`, `:forward`, `:reload`, `:history`, `:url`, `:title`, `:help` and `:quit`.

Theme manifests and selection are available in the engine. The Win32 frontend applies the active theme palette to its custom-painted browser chrome and address/search fields; some native control details and theme-specific layouts are not yet fully customized. On Windows, theme data is stored under `%APPDATA%\\ArchiaBrowser\\themes`.

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
- [x] Local persistent bookmarks and bookmark-bar interaction.
- [x] Persistent browsing history (bounded local history, available after restart).
- [x] Basic HTTP/HTTPS download handling with safe destination naming and overwrite protection (background queue/progress UI still pending).
- [ ] Better error pages and loading feedback.
- [x] Basic persistent settings for search provider, homepage and session restoration.
- [x] Open individual history entries and bookmarks in new tabs, plus batch-open recent history and saved bookmarks with keyboard shortcuts.
- [x] Persistent HTTP User-Agent profiles (Archia, Chromium-compatible and Firefox-compatible; request header only, not a different rendering engine).


### W2 — Theme system
- [x] Platform-independent theme manifest and palette model.
- [x] Built-in Archia, Chrome-inspired, Firefox-inspired and Edge-inspired themes.
- [x] Persistent theme selection per user.
- [x] Install and uninstall custom data-only theme manifests through the engine API.
- [x] Native GUI applies built-in theme colors to browser chrome.
- [ ] Tab placement and sidebar layout follow the selected theme.
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


Use Ctrl+Shift+U or the browser menu to copy all currently open tab titles and URLs as tab-separated text; Ctrl+Alt+U exports the same tabs as Netscape bookmark HTML. Ctrl+S downloads the current page; Ctrl+Shift+R performs a hard reload by clearing only the HTTP response cache first. The bookmark tools can import bookmark data from the Windows clipboard: tab-separated `title<TAB>URL` and `URL<TAB>title` rows are accepted, as are plain HTTP/HTTPS URL lists (one per line). Invalid schemes are ignored and duplicate URLs are removed. Ctrl+Shift+I runs the same bookmark import directly, and Ctrl+Shift+Y copies the browsing history as tab-separated text.


### Presse-papiers et navigation rapide

- `Ctrl+Shift+V` colle le contenu texte du presse-papiers dans la barre d’adresse logique : une URL HTTP(S) est ouverte directement, sinon le texte est recherché avec le moteur configuré.
- `Ctrl+Shift+F` lance cette même navigation dans un nouvel onglet.
- Le menu Archia propose également ces deux actions.
- `Ctrl+Shift+G` copie l’historique au format HTML Netscape, importable dans les navigateurs compatibles.
- Les entrées vides, trop longues, contenant des caractères de contrôle ou utilisant un schéma autre que HTTP(S) sont rejetées.


### Gestion des onglets par le presse-papiers

- `Ctrl+Shift+U` copie les titres et adresses de tous les onglets ouverts au format texte TSV.
- `Ctrl+Alt+U` exporte les onglets ouverts au format HTML Netscape, importable comme favoris dans les navigateurs compatibles.
- `Ctrl+Shift+M` lit les URL ou favoris Netscape/TSV présents dans le presse-papiers et ouvre les liens dans des onglets séparés.
- `Ctrl+Alt+Left` ferme les onglets situés à gauche de l'onglet actif ; `Ctrl+Alt+Right` ferme ceux situés à droite. L'onglet actif est conservé.
- `Ctrl+Alt+Shift+W` ferme tous les autres onglets et conserve uniquement l'onglet actif.
- `Ctrl+Alt+D` et `Ctrl+Shift+D` dupliquent la page actuelle dans un nouvel onglet.
- `Ctrl+Alt+C` copie le titre et l'adresse de la page au format TSV (`titre\tURL`), prêt à coller dans un tableur ou à réimporter.
- `Ctrl+Alt+M` copie la page actuelle sous forme de lien Markdown (`[titre](<URL>)`), pratique pour les notes et les issues.
- `Ctrl+Alt+Shift+M` copie la liste de tous les onglets ouverts au format Markdown, en conservant leur ordre.
- Le menu contextuel d'un onglet propose également de fermer les onglets à gauche ou à droite de celui-ci.
- L'ouverture multiple est plafonnée à 20 liens par opération et respecte la limite globale de 100 onglets.
- Les liens non HTTP(S) sont ignorés et les doublons sont supprimés par le parseur d'import.


Les favoris peuvent aussi être exportés en Markdown depuis le menu Archia ou avec `Ctrl+Alt+Shift+B`. L’import des favoris accepte les liens Markdown HTTP(S) (y compris les URL encadrées par `<...>`), en plus des formats texte, TSV et Netscape HTML.


- `Ctrl+Shift+O` ouvre les favoris enregistrés dans des onglets, dans l’ordre, en respectant la limite de session.
- `Ctrl+Alt+Shift+H` copie l’historique au format Markdown ; le menu Archia propose aussi cette exportation.

### Restauration des onglets fermés

- `Ctrl+Shift+T` rouvre le dernier onglet fermé.
- `Ctrl+Alt+Shift+T` rouvre en une fois les onglets récemment fermés, jusqu’à la limite globale de 100 onglets.
- Le menu Archia propose aussi la restauration groupée.
- `Ctrl+Alt+Shift+R` copie les adresses des onglets récemment fermés au format Markdown ; les entrées vides et les URL non HTTP(S) sont ignorées.


### Restauration individuelle des onglets fermés

- Le menu Archia affiche maintenant les 20 derniers onglets fermés, avec leur hôte et leur chemin.
- Choisir une entrée rouvre précisément cet onglet, sans supprimer les autres entrées de la liste.
- La restauration respecte la limite globale de 100 onglets.


### Nettoyage avancé des onglets

- Le menu contextuel d'un onglet permet de fermer tous les autres onglets du même site (même hôte), en conservant l'onglet choisi.
- `Ctrl+Alt+Shift+X` vide la liste des onglets récemment fermés sans toucher aux onglets actuellement ouverts.
- Le menu Archia propose également « Vider la liste des onglets fermés ».
