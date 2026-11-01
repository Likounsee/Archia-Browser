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

The current desktop application starts in a shared **cross-platform Rust GUI** built with `egui` and `eframe`, rendered through `wgpu`. Windows, Linux and the future ArchiaOS port use the same application UI and browser chrome; the frontend does not embed WebView2, Edge, Chromium or another browser engine. Platform-specific differences are restricted to windowing, GPU surface creation and operating-system integration.

On Windows, from the repository root, run:

```powershell
.\scripts\run-windows.ps1 https://example.org
```

Or start with a blank tab:

```powershell
.\scripts\run-windows.ps1
```

The active chrome is rendered by `egui` on the `wgpu` backend. This avoids Direct2D/DirectWrite and prevents the Windows frontend from becoming a separate implementation from ArchiaOS. `wgpu` selects the appropriate graphics backend per platform while the widgets, layout and interaction logic remain shared.

The shared interface uses Archia's graphite-and-lime visual identity and loads the Inter typeface when it is installed. On Windows, `scripts/run-windows.ps1` installs the official Inter 4.1 font package per user; on Linux and ArchiaOS, run `bash scripts/install-inter-font.sh`. If Inter is not present, egui falls back to its bundled proportional font. The old Win32/GDI frontend is no longer launched or compiled by `main.rs`; its source is temporarily retained only as a feature-parity reference during migration.

The shared frontend now provides the application window, tab selection/creation/closing, Back/Forward/Reload, address/search, a new-tab landing page and display of Archia's existing software-rendered page surface through an `egui` texture. Bookmarks and up to 500 history entries are persisted locally, with dedicated Favoris/Historique menus; a Liens menu exposes detected page links, and the first keyboard shortcuts (Ctrl+L/T/W/R/D and Alt+Left/Right) plus close/duplicate tab context actions are implemented. This is still an intermediate migration, not feature parity with the legacy Windows frontend: session restore, download management, browser preferences, rich page/link/tab context menus, clipboard workflows, the full keyboard-shortcut set, and direct pointer hit-testing of rendered links are still to be ported. The browser engine continues to own URL handling, HTTP/HTTPS transport, HTML/DOM, CSS/layout and page display-list generation; the page bitmap itself is still software-rendered before being uploaded as a wgpu texture. JavaScript and modern-site compatibility remain incomplete.

The same `egui`/`wgpu` desktop frontend is now selected on Windows and non-Windows desktop targets. ArchiaOS can share this UI when its platform layer exposes the required window and GPU surface integration; a native ArchiaOS build and runtime test remain to be completed.

Theme manifests and selection remain available in the engine. The new frontend currently applies the common Archia graphite-and-lime palette; wiring all theme manifests into the egui style system is part of the remaining UI parity work.

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
- [x] Display Archia's software-rendered page surface in the shared egui/wgpu frontend.
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

- Windows and ArchiaOS must share the same desktop interface and browser-chrome implementation.
- Platform-specific code is limited to windowing, GPU surfaces, storage paths and OS integration.
- Keep the browser engine independent from the platform windowing layer.
- Do not embed Chromium, Electron, Gecko, WebKit or Servo.
- Prefer coherent, tested feature batches over isolated cosmetic changes.
- GitHub Actions is the authoritative build/test signal; never report checks as passed unless their result is visible for the exact commit.
- Keep platform-specific code isolated behind a small platform adapter; do not fork the browser UI by operating system.

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
- Le menu Archia permet aussi de copier la liste des onglets ouverts au format Markdown ou CSV ; l'export CSV échappe les guillemets et ignore les URL qui ne sont pas HTTP(S).
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
