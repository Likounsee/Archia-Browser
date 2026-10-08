# Archia Browser

> Part of the Archia ecosystem.

- 🖥️ **ArchiaOS:** https://github.com/Likounsee/ArchiaOS
- 💾 **OpenFS:** https://github.com/Likounsee/OpenFS

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
- [x] Core project structure
- [x] Core memory budget and RAII resource tracking
- [x] Memory pressure state foundation
- [x] Reclaimable resource cache foundation
- [x] LRU-style resource cache eviction and key replacement
- [x] Core event loop foundation
- [x] HTML tokenizer (initial)
- [x] HTML parser / tree builder (initial)
- [x] HTML void-element handling, paragraph/list auto-closing, comment preservation and implicit html/head/body structure
- [x] DOM node model (initial)
- [x] CSS tokenizer (initial)
- [x] CSS declaration parser (initial)
- [x] CSS selector parsing and specificity foundation
- [x] CSS stylesheet rules and basic cascade
- [x] HTML attribute parsing and DOM preservation
- [x] Style resolution for matching DOM nodes
- [x] Attribute selector operators
- [x] `!important` handling and inline style precedence (initial)
- [x] CSS-wide value handling (`initial`, `inherit`, `unset`) and box shorthand expansion
- [x] First block-flow layout engine foundation
- [ ] Standards-compliant layout engine
- [x] Software renderer foundation
- [x] URL parsing and HTTP request/response primitives (initial)
- [x] Native request filtering / ad-blocking foundation
- [x] Native HTTP/1.1 transport foundation (HTTP; HTTPS/TLS pending)
- [x] Bounded response/header memory limits
- [x] Document redirect handling with a bounded redirect count
- [x] Cookie persistence across document redirects
- [x] Cookie domain validation, Secure delivery and Max-Age deletion
- [x] First-party/third-party filtering context
- [ ] JavaScript runtime
- [ ] Browser UI
- [x] Same-origin foundation
- [ ] Full security model
- [ ] ArchiaOS integration
- [ ] Windows release
- [ ] Linux release

## Roadmap

### Phase 1 — Foundation
- [x] Establish the project architecture.
- [x] Add the cross-platform build system.
- [x] Add GitHub Actions CI.
- [x] Define core data structures and error handling.
- [x] Create the first executable entry point.
- [x] Add the first memory-budget and resource-ownership layer.
- [x] Add the first deterministic event-loop layer.

### Phase 2 — HTML and DOM
- [x] Implement the first HTML tokenizer.
- [x] Implement initial tree construction.
- [x] Build the first DOM node model.
- [ ] Add standards-compliant HTML parsing and error recovery.
- [ ] Add document traversal and mutation APIs.
- [x] Add initial HTML attributes and preserve them in the DOM.
- [ ] Add namespaces, text normalization and standards-compliant attribute semantics.

### Phase 3 — CSS and style
- [x] Implement initial CSS tokenization.
- [x] Implement initial CSS declaration parsing.
- [x] Implement selector parsing and specificity.
- [x] Implement stylesheet rules and basic cascade.
- [x] Compute styles for matching DOM nodes.
- [ ] Add selector sibling combinators and full selector semantics.
- [x] Implement initial inheritance and `!important` handling.
- [ ] Implement CSS origins and full cascade semantics.
- [ ] Expand CSS syntax toward standards compliance.

### Phase 4 — Layout and rendering
- Implement box generation.
- Implement block and inline layout.
- Add text layout.
- Build a software renderer.
- Add scrolling and basic viewport handling.

### Phase 5 — Web platform
- [x] Add structured URL parsing foundation.
- [x] Add request filtering foundation for native ad/tracker blocking.
- [x] Implement initial HTTP transport.
- [ ] Add HTTPS/TLS transport.
- [x] Add initial redirects, cookie persistence and resource caching foundations.
- [ ] Add standards-compliant redirects, cookie attributes and HTTP caching.
- [ ] Add forms and navigation.
- [ ] Expand browser security boundaries.

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

## Current engineering priorities

1. Turn the layout foundation into a real block/inline layout engine.
2. Build a platform-neutral software display list and renderer.
3. Build asynchronous HTTP/HTTPS transport behind request policy, starting from the current HTTP/1.1 foundation.
4. Expand HTML parsing toward standards-compliant tree construction.
5. Expand CSS selectors, cascade, inheritance and computed values.
6. Extend first-party/third-party filtering into a complete filter-list and permission model.
7. Keep ad/tracker blocking before transport so blocked resources are not downloaded when policy allows.
8. Keep every subsystem independently testable through GitHub Actions.
