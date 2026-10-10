# P0 remediation roadmap

This checklist tracks release-blocking correctness and security work for Archia Browser.
A percentage is calculated by checklist items marked **Done**, not by lines of code. A
check is only Done after the implementation and regression coverage have been inspected;
execution status is tracked separately and must not be inferred from tests merely existing
in source.

## P0-1 — URL and origin parsing
- [x] Reject malformed URL authorities (invalid host characters, malformed userinfo, ports, and IPv6).
- [x] Ensure origin, cookie, policy, and transport code use consistent host/port interpretation.
- [x] Add regression cases for malformed and valid authority forms.
- [x] Run the URL and networking test suites on CI.

## P0-2 — HTTP request/response framing
- [ ] Reject invalid header names/values, duplicate case-insensitive headers, and ambiguous message framing.
- [ ] Verify Content-Length, Transfer-Encoding, HEAD, interim responses, and response size limits.
- [ ] Add adversarial framing tests and run transport tests.

## P0-3 — Navigation and local-file boundary
- [ ] Ensure remote navigation and subresources cannot reach file URLs through redirects or references.
- [ ] Verify cross-origin redirects strip credentials and sensitive headers and prevent HTTPS downgrade leaks.
- [ ] Check local-file decoding and filesystem access boundaries; add regression tests.

## P0-4 — Cookies and cache isolation
- [ ] Verify cookie domain/path/secure/prefix behavior and bounded cookie storage.
- [ ] Verify cache keys and cache policy do not mix origins, credentials, or blocked requests.
- [ ] Run cookie/cache/loader regression suites.

## P0-5 — Untrusted HTML and resource exhaustion
- [ ] Ensure malformed UTF-8-independent HTML tokenization/tree construction cannot panic.
- [ ] Add input-size/token/node limits or otherwise demonstrate bounded processing for hostile documents.
- [ ] Run HTML/DOM tests and fuzz/adversarial cases where available.

## P0-6 — Memory accounting and failure handling
- [ ] Verify critical allocations are actually governed by budgets and all counters remain consistent.
- [ ] Remove avoidable panic paths in externally reachable processing; distinguish internal invariants from hostile input.
- [ ] Run full workspace tests, formatting, and build.

## Verification and completion rules
- Do not mark a P0 item Done merely because code or a test exists; the targeted tests must pass in CI.
- Track each commit SHA and CI run in the project report.
- Overall P0 progress = Done items / all checklist items. Items without verified results remain open.
- Keep this roadmap updated as new release-blocking defects are found. P1 standards completeness,
  JavaScript, UI polish, and broader web compatibility are outside this P0 checklist unless they
  expose a concrete crash, security boundary failure, or resource-exhaustion issue.

## Current execution log (2026-10-10)

- Created this roadmap on branch \`Archia-Browser\`.
- Added URL-authority checks for ASCII DNS/IPv4 host syntax and percent-encoded userinfo, and regression cases for malformed hosts, malformed userinfo, and valid userinfo with a trailing-dot hostname.
- Related commits: \`a92390e2331ea7d2b842899b19dac79331441e4e\` (roadmap), \`4080baec6e6d039a7934f9514f28397a167da7cb\`, \`aaa7ec90f4e879a21e09d38f0d821deabe2e227a\`, \`a7bf89b1810b1d978b10c7e7992dc54d80f0ac47\`.
- **Verification pending:** this environment cannot resolve GitHub for a local checkout, and the available workflow lookup returned no runs for these commits. Do not mark the URL item Done until formatting, tests, and build pass on CI.

- Continued P0-2: response parsing now rejects duplicate HTTP header names case-insensitively, including non-framing headers such as `Content-Type` and `Set-Cookie`; regression cases added. Commit: `233a3d7c6a54bdd199576572cf1482406d79617f`.
- **Verification still pending:** source-level regression tests are committed, but they have not been executed in this environment. P0-1 and P0-2 remain open until the corresponding CI checks pass.
- Extended P0-2 to reject duplicate chunked trailer fields case-insensitively and added an adversarial regression fixture. Commits: `43afbfebfc9694a4ef863ccd7452678eb0235d6b`, `7f3737d01dabce14ba81c1298d8281d49f780dac`, `ee7afb81c53784778c4a010f1477101648a0955e` (fixture and escape corrections).
- Latest GitHub combined-status lookup returned no status checks for these commits; this is **not** evidence of a passing build. Local compilation/testing remains unavailable here.
- Continued P0-3: browser anchor activation now blocks remote-document navigation to `file://` targets, including when an untrusted `<base href>` points to a local file. Added regression tests for direct links and the base-href case. Commit: `c6836c09251d9d64684afae24a4157d234e30cf8`.
- **Verification pending:** these regression tests are committed but not executed here; P0-3 remains open until CI confirms formatting, tests, and build. No CI status checks were available from the current GitHub status lookup.
- Continued P0-4: cache lookup and storage now refuse URLs containing user-info. Previously the cache key normalized only host/port/path, so a URL such as `https://alice:secret@example.org/account` could alias a public entry before the transport rejected user-info. Added a regression test ensuring it neither reads nor replaces the public entry. Commit: `d704110bffd465270b747ade1b4997faeffd11ec`.
- **Verification pending:** no test execution or successful CI run has been confirmed in this session; P0-4 remains open.


- **P0-1 completed on 2026-10-10:** hardened URL authority parsing already in place, rejected ambiguous legacy numeric IPv4 spellings, aligned effective-port semantics with HTTP/HTTPS defaults, and canonicalized equivalent IPv6 literal spellings so origin/cookie/policy/transport consumers see the same host. Added URL and Origin regression tests. Commits: `21ac72fa7d52a4286e780959dc9c67ff86cace61`, `2d643e276b368094843fa2642c60d1542667efd3`, `58cad9b6e16e869914abbc5c136af80cdc5e043d`, `86af7a2a830f20acc1d4b99be58ba0b701491b61`. Final CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38060115071 — formatting, workspace tests (439 passed, 0 failed), and workspace build succeeded. An intermediate formatting-only failure was corrected in the final commit.


- **P0-1 security follow-up audit (2026-10-10):** source review found two origin-comparison edge cases after the initial completion entry: all `file://` URLs previously collapsed to an empty-host tuple origin, and non-HTTP schemes treated absent ports and explicit port `0` as equal via a sentinel. Fixed by assigning independently created file origins distinct opaque identities (clones retain identity) and comparing optional effective ports without a `0` sentinel. Added regression tests. Commit: `325809e4d80911b2d6696f6a5ba91103fa322df9`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38060504305 — formatting, workspace tests (442 passed, 0 failed), and workspace build succeeded. This verifies the targeted regressions, not a formal proof of absence of all URL/origin vulnerabilities; P0-1 remains subject to review if new cases are discovered.

- **P0-1 adversarial URL parsing follow-up (2026-10-10):** a further review found two path interpretation gaps: literal backslashes could be treated as separators by downstream consumers while remaining ordinary characters in this parser, and absolute URLs retained literal/percent-encoded dot segments that servers or local-file handling could normalize differently from policy/cache code. The parser now rejects backslashes in URLs and references, and canonicalizes literal and encoded dot segments during parsing as well as relative resolution. Added regression cases for backslash rejection and `.`, `..`, `%2e`, `.%2e`, `%2e.`, and `%2e%2e` path forms. Final code commit: `9a8135b26dd07cb971cfbf2bc8b92717500e5566`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38061150849 — formatting passed, workspace tests (444 passed, 0 failed), and workspace build succeeded. This is source review plus regression coverage, not a formal proof or a substitute for future fuzzing; P0-1 stays open to further findings.

- **P0-1 loader/origin consistency follow-up (2026-10-10):** the cross-consumer audit found that `DocumentLoader` still used the legacy numeric `effective_port()` sentinel for origin checks and treated all `file://` URLs as one tuple origin, bypassing the stricter semantics in `security::Origin`. Loader origin comparison now preserves absent-vs-zero ports and conservatively treats file URLs as opaque/cross-origin because a `Url` value does not carry document opaque-origin identity; referrer generation is suppressed whenever either side is a file URL. Added regression tests for file-to-file origin/referrer handling and absent-vs-zero custom-scheme ports. Code commit: `e5f1d4280e4e2a00db846f80ed93da6d6a41f98d`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38061488168 — formatting passed, workspace tests (446 passed, 0 failed), and workspace build succeeded.

- **P0-2 framing follow-up (2026-10-10):** request and response framing used Rust's Unicode-aware `str::trim()` for `Content-Length` and response `Transfer-Encoding`. That could accept non-ASCII whitespace around framing values even though those UTF-8 bytes remain on the wire and are not HTTP optional whitespace (which is only SP/HTAB). Added `trim_http_ows` and applied it consistently to request Content-Length, response Content-Length, response Transfer-Encoding, and parsed header values; regression tests cover NBSP rejection and valid ASCII OWS. Commit: `1a401f39334fbd2aeb6a1ce6012e73cd45769c70`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38061866571 — formatting passed, workspace tests (447 passed, 0 failed), and workspace build succeeded. This is one targeted P0-2 finding; P0-2 remains open pending the rest of the framing audit.

- **P0-2 bodyless-response framing follow-up (2026-10-10):** response parsing previously skipped numeric `Content-Length` validation for `HEAD`, `204`, `205`, and `304` responses because the no-body branch returned before parsing the field. It now validates the syntax even when the body is forbidden, while still allowing valid representation lengths on `HEAD`/304 without requiring those bytes to be present. Added malformed and non-ASCII-whitespace regression cases. Code commit: `f7a9339466ddb88c18840225f0a618bdad09634c`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38062144820 — formatting passed, workspace tests (448 passed, 0 failed), and workspace build succeeded. P0-2 remains open for the remaining request/response framing review.

- **P0-2 status-specific framing follow-up (2026-10-10):** informational (1xx) responses were skipped without rejecting `Content-Length` or `Transfer-Encoding`, and final `204 No Content` responses accepted those forbidden framing fields. The parser now rejects either field on 1xx and 204 responses; regression cases cover both headers and the no-content baseline now uses a compliant 204. Code commit: `0c581ea8fa412fbc5e1dfe907d94b8b34c9a7d5e`. CI: https://github.com/Likounsee/Archia-Browser/actions/runs/38062308573 — formatting passed, workspace tests (449 passed, 0 failed), and workspace build succeeded. P0-2 remains open pending a broader framing review.
