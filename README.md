# Archia Browser

> A homegrown, cross-platform web browser engine for the Archia ecosystem.

- 🖥️ **ArchiaOS:** https://github.com/Likounsee/ArchiaOS
- 💾 **OpenFS:** https://github.com/Likounsee/OpenFS

![Status](https://img.shields.io/badge/status-active%20development-orange)
![Target platforms](https://img.shields.io/badge/targets-ArchiaOS%20%7C%20Windows%20%7C%20Linux-blue)
![Engine](https://img.shields.io/badge/engine-homegrown%20%7C%20no%20Chromium%2FGecko%2FWebKit-green)

Archia Browser is a lightweight browser-engine project built from scratch. The engine does not embed Chromium/Blink, Gecko, WebKit or Servo. ArchiaOS, Windows and Linux are target platforms, not a claim that a complete end-user browser UI or production-ready release is available on each platform.

## Current progress

These percentages are engineering estimates of **functional coverage**, not lines of code. They measure the current implementation against the scope required for a serious browser engine.

| Subsystem | Progress | Current state |
|---|---:|---|
| Foundation / architecture | ~90% | Memory budgets, resource cache, event loop, modular layers and CI |
| HTML tokenizer/parser | ~62% | Working tree builder with void elements, implicit `tbody`, nested-form recovery, RCDATA/plaintext handling, bogus-comment handling for unknown declarations, implied structure, text-node coalescing and null replacement; full HTML5 tree construction remains incomplete |
| DOM | ~53% | Node/attribute model, iterative preorder descendant traversal, id/class lookup and mutation primitives; namespaces and a complete standards DOM remain |
| CSS tokenizer/parser | ~60% | Declarations, shorthands, growing value syntax and decoded CSS escapes in identifiers/hash tokens |
| CSS selectors | ~65% | Specificity, attributes, siblings, form-state and structural pseudo-classes |
| Cascade / computed style | ~55% | Inheritance, !important, CSS-wide keywords and custom properties/var() |
| Block/inline layout | ~50% | Flow, wrapping, margins, padding, borders, line-height, alignment and positioning foundations |
| Box model / dimensions | ~60% | Percentages, calc(), border-box, min/max constraints and auto margins |
| Positioning | ~30% | Relative, absolute and fixed initial behavior |
| Software rendering | ~38% | Display list, backgrounds, borders, colors, bitmap text and clipping |
| Text / fonts / shaping | ~10% | Bootstrap glyph rasterizer only |
| HTTP / networking | ~55% | HTTP/1.1, redirects, request policy, cookies and bounded caching |
| HTTPS / TLS | ~45% | Native `rustls` transport with certificate-chain and hostname validation; broader end-to-end integration and security coverage remain incomplete |
| Navigation / history | ~70% | Back/forward, URL resolution, reload, titles and link activation |
| Tabs / session state | ~55% | Independent tab histories and active page retention |
| Forms | ~35% | Initial GET/POST URL-encoded submission |
| Security | ~20% | Same-origin foundation and request policy; isolation/sandboxing still pending |
| JavaScript | ~0% | Runtime not started |
| Browser UI | ~5% | Browser/tab APIs exist; full window UI remains |
| ArchiaOS integration | ~5% | Platform abstraction exists |
| Tests / CI | — | Latest verified green commit: `cd5a323e5d338f032b1d4b4f590339cdc0fd7b27`, run `38439360131` on Ubuntu, Windows and macOS; formatting, workspace tests, build and Clippy passed, and Linux RustSec/license audits passed. The 24-hour fuzz campaign, unsupported platforms and broader standards-conformance coverage remain unverified. |

**Overall engine foundation:** roughly **40–45%** as a non-reproducible engineering estimate, not a measured compatibility score. It is not test coverage or standards conformance. TLS is implemented at the transport layer, but that does not mean the whole network stack is production-ready.

## Vision

The long-term stack is:

- standards-oriented HTML and DOM;
- CSS parsing, cascade and computed values;
- block, inline, flex and other layout modes;
- real font loading and text shaping;
- software rendering with optional GPU acceleration;
- HTTP/HTTPS, cookies, cache and resource policy;
- JavaScript and Web APIs;
- browser security, isolation and sandboxing;
- tabs, windows, navigation, history, downloads and settings;
- developer tools;
- native ArchiaOS integration.

The project prioritizes **real web-platform behavior over browser-specific shortcuts**.

## Architecture

The repository is deliberately split into independent subsystems:

```text
Archia-Browser/
├── src/
│   ├── core/       # memory, event loop, navigation, tabs
│   ├── html/       # tokenizer, parser, DOM
│   ├── css/        # tokenizer, selectors, cascade
│   ├── net/        # URL, policy, HTTP, cookies, cache
│   ├── layout.rs   # block/inline layout and box model
│   ├── render.rs   # display list and software painting
│   ├── surface.rs  # RGBA software surface
│   ├── security/   # origin/security foundations
│   └── platform/   # platform abstraction
├── tests/
├── docs/
└── .github/workflows/
```

The implementation structure may evolve as the engine grows, but renderer and platform assumptions must not leak into HTML parsing.

## Implemented foundations

### HTML / DOM
- Initial HTML tokenization and tree construction.
- Implicit `tbody` insertion for table rows without an explicit section.
- Ignore nested form start tags while a form is open.
- Tokenize `title`/`textarea` as RCDATA and `plaintext` through EOF.
- Treat unknown declarations, processing-instruction-like markup and CDATA syntax outside foreign content as bogus comments.
- Void elements.
- Implicit html/head/body structure.
- Paragraph and list auto-closing rules.
- Comment preservation and adjacent text-node coalescing.\n- Literal U+0000 replacement with U+FFFD in normal, attribute and raw-text input.
- Attribute preservation and case-insensitive lookup/removal.
- Iterative preorder descendant traversal and subtree element lookup (including id/class queries).
- Child insertion, removal, replacement and indexed access.
- Anchor href extraction.
- HTML `hidden` handling in layout.

### CSS
- CSS tokenization and declarations.
- Selector matching and specificity.
- Attribute operators and selector flags.
- `+` and `~` sibling combinators.
- Form-state pseudo-classes.
- `:not()`, `:is()`, `:where()`.
- `:first-of-type`, `:last-of-type`, `:only-of-type`, `:nth-of-type()`, `:nth-last-of-type()`.
- `:nth-child(odd/even)`.
- `!important` and inline-style precedence.
- CSS-wide keywords.
- Box shorthand expansion.
- CSS custom properties and inherited `var()` resolution with cycle protection.
- Case-insensitive property names.

### Layout
- Block and inline flow.
- HTML-aware default display mapping.
- Inline wrapping and `<br>` line breaks.
- Percentage width/margin/padding.
- `calc()` with px and percentage arithmetic.
- Border-box sizing and min/max constraints.
- Fixed-width auto-margin distribution.
- Adjacent block margin collapsing.
- Relative, absolute and fixed positioning foundations.
- Unitless and percentage line-height.
- `white-space: nowrap` / `pre` wrapping suppression.
- Initial `text-align` center/right/end/justify support, including non-final-line distribution.
- `display: flow-root` recognition.

### Rendering
- Platform-neutral display list.
- RGBA software surface.
- Background and padding-box painting.
- Solid borders and border shorthand.
- Named, hexadecimal, RGB and RGBA colors.
- Visibility suppression.
- Nested clipping for `overflow: hidden`, `clip`, `auto` and `scroll`.
- Bootstrap bitmap glyph rasterization.

### Networking / browser state
- Structured URL parsing.
- Local file transport.
- HTTP/1.1 transport with bounded response/header memory.
- Request filtering before transport.
- Redirect handling with bounded redirect count and method semantics.
- Cookie persistence, Domain validation, Secure delivery and Max-Age deletion.
- First-party/third-party request context.
- Bounded HTTP response cache with conservative reuse/storage rules.
- Navigation history and relative URL resolution.
- Document titles and loaded-page URL propagation.
- `<base href>` resolution.
- Anchor navigation and `target="_blank"`.
- Initial URL-encoded form submission.
- Per-tab history and active Page retention.

## Roadmap

### Phase 1 — Foundation
- [x] Architecture, build system and CI.
- [x] Memory/resource ownership and cache foundation.
- [x] Event loop and core error handling.

### Phase 2 — HTML / DOM
- [x] Initial tokenizer, tree builder and DOM.
- [x] Initial mutation and lookup APIs.
- [ ] Standards-compliant HTML parsing and error recovery.
- [ ] Full document traversal model.
- [ ] Namespaces, text normalization and standards-compliant attributes.

### Phase 3 — CSS / style
- [x] Initial tokenizer, declarations, selectors and cascade.
- [x] Inheritance, CSS-wide keywords, !important and custom properties.
- [ ] CSS origins and complete cascade ordering.
- [ ] Broader CSS value grammar.
- [ ] Generated content and more pseudo-elements.
- [ ] More standards-compliant computed values.

### Phase 4 — Layout / rendering
- [x] Initial block and inline layout.
- [x] Box model, percentages, calc(), min/max and positioning foundations.
- [x] Initial text alignment and clipping.
- [x] Initial inline vertical alignment (`top`, `middle`, `bottom`, `text-top`, `text-bottom` and pixel offsets).
- [ ] Inline-block intrinsic sizing.
- [ ] Flex layout.
- [ ] Tables and table layout.
- [ ] Overflow scrolling and viewport scrolling.
- [ ] Stacking contexts and z-index.
- [ ] Images and replaced-element sizing.
- [ ] Border radius, opacity and transforms.
- [ ] Real font loading, shaping and glyph rasterization.

### Phase 5 — Web platform / networking
- [x] URL, policy, HTTP/1.1, redirects, cookies and cache foundations.
- [x] Native HTTPS/TLS via `rustls` (transport implementation exists; end-to-end integration/security coverage is still incomplete).
- [ ] More complete HTTP caching semantics.
- [ ] Complete cookie attributes and SameSite behavior.
- [ ] Persistent storage and resource lifecycle improvements.
- [ ] Expanded security boundaries and isolation.

### Phase 6 — JavaScript / Web APIs
- [ ] Lexer/parser.
- [ ] Bytecode/interpreter or equivalent runtime.
- [ ] Objects, functions, prototypes and built-ins.
- [ ] DOM bindings.
- [ ] Events and timers.
- [ ] Fetch and browser Web APIs.
- [ ] Task/microtask scheduling.

### Phase 7 — Browser product
- [x] Initial Browser, tab and navigation state APIs.
- [ ] Window and tab UI.
- [ ] Address bar.
- [ ] History/bookmarks UI.
- [ ] Downloads.
- [ ] Settings.
- [ ] Developer tools.
- [ ] Private browsing.

### Phase 8 — ArchiaOS / production
- [ ] Native ArchiaOS integration.
- [ ] Process isolation and sandboxing.
- [ ] GPU acceleration.
- [ ] Performance profiling and optimization.
- [ ] Packaging and distribution.
- [ ] Windows release.
- [ ] Linux release.

## Engineering priorities

1. Complete the inline formatting context and improve layout correctness.
2. Implement flexbox and additional layout modes.
3. Replace bootstrap text with real font loading and shaping.
4. Add scrolling, stacking contexts and richer painting.
5. Implement HTTPS/TLS without bypassing request policy.
6. Continue HTML/CSS standards compliance.
7. Start the JavaScript runtime and DOM integration.
8. Expand security isolation before exposing untrusted active content.
9. Keep every subsystem independently testable through GitHub Actions.

## Development rules

- Build the browser engine; do not embed another browser engine.
- Keep HTML, DOM, CSS, layout, rendering, networking, security and platform layers separated.
- Treat GitHub Actions as the authoritative validation environment.
- Prefer small, test-backed standards-oriented changes.
- Never hide an implementation problem behind a test-only workaround.

## What Archia Browser is not

Archia Browser is not:
- a Chromium/Blink wrapper;
- an Electron application;
- a Firefox/Gecko wrapper;
- a WebKit wrapper;
- a Servo-based browser;
- a UI shell around another browser engine.

## License

License to be defined.
