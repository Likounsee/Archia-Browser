# Archia Browser

![Status](https://img.shields.io/badge/status-early%20development-orange)
![Platforms](https://img.shields.io/badge/platforms-ArchiaOS%20%7C%20Windows%20%7C%20Linux-blue)
![Engine](https://img.shields.io/badge/engine-homegrown%20%7C%20no%20Chromium%2FGecko%2FWebKit-green)

**Archia Browser** is a lightweight, cross-platform web browser designed for **ArchiaOS**, while also targeting **Windows** and **Linux**.

The core objective is simple: build the browser ourselves.

## Vision

Archia Browser is intended to become a complete browser stack developed inside this project, rather than embedding an existing browser engine.

The long-term stack includes:

- HTML tokenizer and parser
- DOM implementation
- CSS tokenizer and parser
- CSS cascade and computed styles
- Layout engine
- Text and graphics rendering
- Networking and HTTP/HTTPS integration
- Cookie and cache systems
- JavaScript runtime
- Web APIs
- Security and sandboxing
- Browser UI and tab management
- Developer tools
- Optional GPU acceleration
- ArchiaOS integration

## Independence

Archia Browser will **not** use Chromium/Blink, Gecko, WebKit, or Servo as its browser engine.

The project may use operating-system APIs and carefully selected low-level libraries where appropriate, but the browser engine itself is developed as part of Archia Browser.

## Targets

| Platform | Goal |
|---|---|
| ArchiaOS | Primary platform |
| Windows | Supported |
| Linux | Supported |

Cross-platform code should remain separated from platform-specific implementations whenever possible.

## Architecture

The project will be built as independent components so that each part can be developed and tested without turning the browser into a monolithic codebase.

Planned high-level structure:

```text
Archia-Browser/
├── README.md
├── LICENSE
├── .github/
│   └── workflows/
├── docs/
├── src/
│   ├── core/
│   ├── html/
│   ├── dom/
│   ├── css/
│   ├── layout/
│   ├── render/
│   ├── net/
│   ├── javascript/
│   ├── security/
│   ├── ui/
│   └── platform/
├── tests/
└── examples/
```

The exact implementation structure can evolve as the engine grows.

## Development principles

1. **Build, don't embed** — the browser engine belongs to this project.
2. **Modular by design** — HTML, CSS, DOM, layout, rendering, networking and JavaScript remain separable.
3. **Cross-platform first** — ArchiaOS, Windows and Linux are first-class targets.
4. **Lightweight** — avoid unnecessary dependencies and background services.
5. **Security first** — untrusted web content must be isolated as the architecture matures.
6. **Standards-oriented** — progressively implement real web standards instead of creating a browser-specific HTML/CSS dialect.
7. **CI as authority** — builds and automated tests should run through GitHub Actions.

## Current status

The repository is at the beginning of development.

Current milestone:

- [x] Project created
- [x] Project vision defined
- [x] Homegrown engine requirement defined
- [ ] Core project structure
- [ ] HTML tokenizer
- [ ] HTML parser
- [ ] DOM
- [ ] CSS tokenizer/parser
- [ ] Style system
- [ ] Layout engine
- [ ] Software renderer
- [ ] Networking
- [ ] JavaScript runtime
- [ ] Browser UI
- [ ] Security model
- [ ] ArchiaOS integration
- [ ] Windows release
- [ ] Linux release

## Roadmap

### Phase 1 — Foundation
- Establish the project architecture.
- Add the cross-platform build system.
- Add GitHub Actions CI.
- Define core data structures and error handling.
- Create the first executable entry point.

### Phase 2 — HTML and DOM
- Implement HTML tokenization.
- Implement tree construction.
- Build the DOM.
- Add document traversal and mutation APIs.

### Phase 3 — CSS and style
- Implement CSS tokenization.
- Implement CSS parsing.
- Implement selectors and cascade.
- Compute styles for DOM nodes.

### Phase 4 — Layout and rendering
- Implement box generation.
- Implement block and inline layout.
- Add text layout.
- Build a software renderer.
- Add scrolling and basic viewport handling.

### Phase 5 — Web platform
- Implement HTTP/HTTPS integration.
- Add URLs, redirects, cookies and caching.
- Add forms and navigation.
- Start implementing browser security boundaries.

### Phase 6 — JavaScript
- Implement the JavaScript language/runtime progressively.
- Connect JavaScript to the DOM.
- Add the first Web APIs.
- Build an event loop and task model.

### Phase 7 — Browser product
- Tabs and windows.
- Address bar.
- History and bookmarks.
- Downloads.
- Settings.
- Developer tools.
- Private browsing.

### Phase 8 — ArchiaOS
- Native ArchiaOS integration.
- Platform services.
- Performance optimization.
- Sandboxing and process isolation.
- Packaging and distribution.

## What Archia Browser is not

Archia Browser is **not** intended to be:

- a Chromium wrapper;
- an Electron application;
- a Firefox/Gecko wrapper;
- a WebKit wrapper;
- a Servo-based browser;
- a UI shell around another browser.

## Contributing

The project is currently developed around the Archia Browser architecture and its long-term goal of a homegrown engine.

Contributions should preserve the project's independence, modularity, security and cross-platform goals.

## License

License to be defined.
