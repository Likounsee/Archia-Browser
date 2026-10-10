# Archia Browser — Security Model (P0 draft)

Status: **draft, not independently reviewed**. This document records the current intended trust boundaries and known gaps; it is not a security certification. Update it whenever navigation, resource loading, scripting, storage, or platform integration changes.

## 1. Scope and current capability boundary

Archia Browser is a Rust browser implementation with a custom URL parser, HTTP/1.1/TLS transport, local-file transport, request policy pipeline, document loader, cookie jar, HTTP cache, HTML/CSS parsers, layout, and software renderer.

The current implementation does **not** provide a usable JavaScript runtime or a complete set of browser Web APIs. Do not treat DOM/CSS support as permission to execute untrusted scripts. New active capabilities must not be enabled merely because a parser or API type exists.

## 2. Assets to protect

- Local files and paths supplied by the operating system.
- Session cookies, authorization headers, referrers, and other request metadata.
- Origin-scoped content and cached responses.
- Process availability and bounded CPU/memory use while processing remote input.
- Integrity of the URL, redirect, request-policy, cache, and cookie decisions.
- User-visible page content and renderer memory safety.

## 3. Trust boundaries

| Boundary | Untrusted input / actor | Required invariant |
|---|---|---|
| Navigation → URL parser | Address bar, links, redirects, caller-created URLs | Reject malformed or ambiguous URLs before policy or transport interprets them. |
| Network → HTTP parser | Remote server, proxy, intermediary | Enforce strict framing, header syntax, body limits, and fail closed on ambiguity. |
| HTTPS → TLS | Remote endpoint and certificate chain | Validate certificate chain and server name; TLS errors must fail closed. |
| Document → subresources | HTML `base`, stylesheet links and CSS imports | Reapply policy to every request and redirect hop; cap depth, request count, and retained CSS bytes. |
| Remote document → local files | Redirects, `base`, stylesheet references | Remote content must not cause local-file reads. |
| Origin → cookies | `Set-Cookie` and request context | Enforce domain/path/secure/SameSite scope and global/per-domain storage limits. |
| Request → HTTP cache | Request headers and cache directives | Policy runs before cache use; credentialed requests and responses with `Vary` are not shared through the current cache. |
| HTML/CSS → DOM/layout/render | Malformed or adversarial markup, styles and dimensions | Bound input, tokens, nesting, selector complexity, resource requests, and output sizes; errors must not panic. |
| Browser UI/platform → privileged actions | Future navigation controls, downloads, permissions and script APIs | Keep privileged operations outside untrusted document control and require explicit, auditable policy checks. |

## 4. Current controls present in source

These controls are implemented in source but still require ongoing regression testing and independent review:

- URL parsing validates authority/ports and percent encoding; file paths reject encoded separators and encoded Windows drive-colon ambiguity. Origin comparisons are centralized; file URLs are treated as opaque for same-origin checks.
- HTTP transport validates request headers and framing, rejects conflicting `Content-Length` / `Transfer-Encoding`, limits headers and response bytes, validates chunking/trailers, and rejects protocol upgrades. Regression tests now verify exact response-body/header-size boundaries on Linux and Windows (`985391947de0fa7ab057a65c210fdffda1a25d30`); coverage of every individual limit at N−1/N/N+1 remains open.
- HTTPS uses rustls with trusted roots and server-name validation.
- Document redirects are bounded; cross-origin redirects remove arbitrary/credential-bearing headers; remote-to-file redirects are rejected.
- Local file transport opens the file before inspecting its handle, requires a regular file, bounds file size, rejects malformed percent escapes and NUL; Windows UNC and drive-relative paths are rejected.
- Request policy is checked before cache hits and network I/O. Stylesheet loads enforce mixed-content and remote-to-file restrictions on every redirect hop.
- Cookie storage is bounded and implements domain/path, secure, HttpOnly, prefix, expiration and SameSite checks.
- HTML, CSS, stylesheet imports and network/local-file bodies have explicit limits. The software RGBA surface rejects allocations over 64 MiB and uses fallible reservation; the display list caps retained commands at 200,000 and retained text at 4 MiB, with an observable truncation flag. Memory-budget tests cover failed reservations, 1,000 balanced reservation cycles, and concurrent attempts not exceeding the configured budget. These are local limits, not proof of a global allocation budget. CI checks formatting, tests, build and Clippy on Linux and Windows; RustSec dependency auditing runs on Linux, and Dependabot checks Cargo and GitHub Actions dependencies weekly. Latest verified CI for `8d9be8576335697dc21bb1ae3767342b1ef2c5f3` is run `38091475251` (Linux and Windows success). Recent panic hardening replaces impossible-state `expect`/`unreachable!` paths in HTML normalization, flex layout, URL normalization and CSS selector parsing with defensive fallback or parse rejection; poisoned loader/pool locks fail closed or recover only the simple `Vec<Arc<_>>` invariant. The resource manager also has a regression test for reservation cleanup on ID exhaustion.

## 5. Known gaps / risks not yet closed

- No process sandbox or privilege separation is established.
- No complete Same-Origin Policy enforcement for script-readable data, CORS, CSP, permissions, downloads or a JavaScript runtime is present.
- The custom URL parser and origin model are not a full WHATWG URL implementation; compatibility and differential testing remain necessary.
- TLS policy/root lifecycle and positive/negative certificate integration tests need a documented, repeatable validation plan.
- Memory accounting is not proven to cover every allocation in DOM, layout, display lists, raster surfaces and all temporary parser buffers.
- Seven cargo-fuzz targets exercise URL parsing/resolution, HTTP response framing, HTML tokenization/DOM parsing, CSS tokenization, stylesheet parsing, selector parsing and cookie parsing/selection. Initial seed corpora are versioned for URL, HTTP, HTML, CSS and cookies; bounded fuzz runs have passed on earlier commits. Extended campaign run `38091891786` was launched on branch `Archia-Browser` with seven parallel 3.5-hour target jobs (24.5 target-hours planned), 10-second per-input timeout and a 4 GiB RSS limit; it was still building targets at the last check and is not yet validation evidence. The workflow builds targets, runs bounded fuzz sessions and uploads crash artifacts on failure. All seven targets passed bounded run `38086530143` on `985391947de0fa7ab057a65c210fdffda1a25d30`, not the current candidate. A current-SHA fuzz run, 24-hour sustained fuzzing, minimized crash-regression corpus and broad WPT/differential coverage remain unverified.
- Dependency advisories, license inventory and reproducible-build policy require a recorded audit.
- A passing unit/CI run proves only the tested revision and covered cases; it does not prove the absence of vulnerabilities.

## 6. Rules for future active features

1. Do not expose untrusted JavaScript, script-readable cookies, cross-origin response data, downloads, or privileged APIs until their policy and isolation model is implemented and tested.
2. New resource types must have explicit policy mapping and per-resource limits before they can be loaded.
3. Every redirect hop must re-check scheme restrictions, origin changes, credentials/referrer handling, and request policy.
4. Every cache optimization must prove that it cannot bypass current policy or share private responses.
5. Add a regression test for each security fix and retain minimized reproductions for parser/transport failures.
6. Do not mark P0 complete until the roadmap exit gate is met: CI green on the candidate SHA, documented dependency review, adversarial/fuzz evidence, and an explicit residual-risk review.

## 7. Review checklist

- [ ] Review each trust boundary against the actual call graph.
- [ ] Verify every control listed above still exists and is covered by a test.
- [ ] Record owners/decisions for accepted residual risks.
- [ ] Revisit this document before adding JavaScript, Web APIs, downloads, extensions, or persistent profiles.
