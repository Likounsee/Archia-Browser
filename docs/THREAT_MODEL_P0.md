# Archia Browser — P0 threat model (draft)

Status: implementation-oriented draft; **not a security certification or completed independent review**.
Scope: current Rust browser prototype on branch `Archia-Browser`. This document records trust boundaries and required controls before untrusted scripts or broad Web APIs are enabled.

## Assets to protect

- Process integrity and availability: no remote input should cause an unbounded allocation, stack exhaustion, deadlock, or uncontrolled work.
- Local user data: files, local URLs, cookies, credentials, history, cache contents, and future profile data.
- Network credentials and origin isolation: cookies, Authorization, referrers, custom headers, and cached representations must not cross an unintended boundary.
- Policy integrity: a blocked resource must not be fetched through redirects, cache hits, CSS imports, or an alternate resource-loading path.
- User intent: privileged browser UI and future downloads/permissions must not be controlled by untrusted document content.
- Build integrity: dependency versions, licenses, advisories, and reproducible CI evidence.

## Actors and trust levels

| Actor/input | Trust assumption | Required treatment |
|---|---|---|
| Remote HTTP(S) server | Untrusted, may be malicious or compromised | Validate framing, headers, redirects, TLS identity, status and content limits |
| HTML/CSS/URL response bytes | Fully attacker-controlled | Parse with bounded work/memory; never rely on well-formed input |
| Redirect target and `<base>` URL | Attacker-controlled navigation/resource destination | Resolve consistently; re-run policy at every hop; forbid remote-to-file and secure-to-cleartext downgrade |
| Cookies and cache directives | Untrusted protocol input and state | Parse conservatively; enforce scope, limits, freshness, and non-leakage |
| Local file URL/path | Sensitive local capability, not inherently safe because it is local | Reject traversal/remote authorities/unsafe path forms; keep file origins isolated; never permit remote content to redirect into files |
| User and browser chrome | Trusted control plane | Keep privileged UI actions and state separate from document-controlled data |
| Dependencies/toolchain | External supply-chain risk | Pin via lockfile, audit advisories/licenses, build with locked dependencies, review updates |
| Future JavaScript/Web APIs | Hostile by default | Do not expose until origin checks, policy, lifecycle, quotas and privilege separation are enforced |

## Trust boundaries and required invariants

1. **URL parser → every consumer**: one parsed representation must drive transport, redirects, origin checks, cookies, cache keys and local-file access. Consumers must not reinterpret raw strings independently.
2. **Document → resource loader**: every document, stylesheet, import and future active resource is checked by policy before cache access and before network transport. Denial must be terminal, not a cache/network fallback.
3. **Network → HTTP parser**: headers, bodies, chunk sizes, trailers, redirects and decoded content are bounded. Conflicting or malformed framing fails closed.
4. **Origin A → origin B**: credentials and arbitrary custom headers are not forwarded by default. Cookie sending follows scheme/domain/path/Secure/HttpOnly/SameSite policy. Cache entries cannot be reused across credentials or request contexts unless correctly partitioned.
5. **Remote content → local filesystem**: remote redirects and resource URLs cannot access `file:`; file URL parsing and OS path conversion must prevent traversal, UNC escape and symlink/TOCTOU mistakes.
6. **Untrusted input → memory/CPU**: limits must cover not only input bytes but object counts, nesting, pixels, retained text, network concurrency and expansion (e.g. CSS imports). Reservations must be released on every error/cancellation path.
7. **Document → browser UI**: document strings are data, never privileged commands. Future UI, clipboard, downloads, permission prompts and profile operations require explicit trusted interfaces.
8. **Build inputs → release artifact**: the candidate SHA must pass formatting, tests, build, lint and dependency checks on supported CI platforms; long fuzz evidence must refer to the candidate or be clearly marked historical.

## Existing controls observed in the codebase

These are code-presence observations, not a claim that all cases are complete:
- HTTP(S) transport uses TLS validation; request/body/header bounds and strict chunked-trailer checks exist.
- Redirect handling limits hops, detects loops, blocks HTTPS→HTTP and remote→file transitions, and strips sensitive/custom headers cross-origin.
- URL and file handling has tests for encoded separators/dot segments, remote file authorities and Windows path edge cases.
- Cookie handling includes scheme restrictions, size/count limits, SameSite/Secure/HttpOnly and cookie-prefix checks.
- HTTP cache rejects credentialed requests, Set-Cookie responses and several private/no-store/no-cache cases; cache-only misses return 504 without transport.
- HTML/CSS parsing, resources, layout and paint surfaces have local bounds; panic-hardening tests exist.
- Cargo.lock and CI checks for formatting, tests, build, Clippy and dependency auditing are configured.

## Known gaps before P0 can close

- Complete a call-site audit proving every sensitive origin comparison uses the common origin API; add integration tests for each consumer.
- Complete a URL-consumer matrix across policy, redirects, transport, cache, cookies, document base URLs, CSS imports and file access.
- Expand TLS integration tests for expired certificates, wrong hostname, unknown roots, invalid chains and failure without fallback.
- Verify HTTP framing boundaries and ambiguous/duplicate headers with N−1/N/N+1 and malformed-response tests.
- Add integration tests proving blocked requests never reach transport, including cache hits, redirects and CSS imports.
- Demonstrate effective global quotas and reservation cleanup across loader, DOM, CSS, layout, display list, raster surfaces and cache; local caps alone are insufficient.
- Finish panic/error/cancellation audit with adversarial tests and retained regression cases.
- Verify RustSec and license audit results on the final candidate SHA and resolve or document all high/critical findings.
- Run the long fuzz campaign only at the final validation stage, then review crashes and preserve minimized regression inputs.
- Publish a residual-risk review and explicitly list unsupported capabilities. JavaScript and broad Web APIs remain out of scope until their security prerequisites are implemented.

## Abuse cases to retain as regression tests

- Malformed HTTP framing, conflicting length/transfer encoding, oversized headers/body/trailers, premature EOF.
- Redirect loops, HTTPS downgrade, remote-to-file redirect, cross-origin credential/custom-header forwarding.
- Cache-Control with malformed quotes, private/no-store/no-cache, only-if-cached miss, personalized response leakage.
- Cookie prefix violations, insecure overwrite/deletion of Secure cookies, SameSite=None without Secure, non-HTTP schemes.
- Encoded path separators/dot segments, UNC and drive-relative paths, origin confusion and file URL aliasing.
- Deep or huge HTML/CSS, pathological selector/import expansion, zero-byte resource exhaustion, allocation failure and poisoned-lock paths.

## Review checklist

- [ ] Security owner reviews each trust boundary and accepts residual risks.
- [ ] Each invariant links to one or more automated tests and CI evidence.
- [ ] All P0 blocking issues have an owner and disposition.
- [ ] Candidate SHA is recorded with green CI and dependency audit.
- [ ] Final long-fuzz run is complete; crashes are triaged and minimized.
- [ ] No untrusted script/runtime/Web API is enabled before the P0 exit gate.
