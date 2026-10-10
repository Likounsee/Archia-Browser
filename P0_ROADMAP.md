# P0 remediation roadmap

This checklist tracks release-blocking correctness and security work for Archia Browser.
A percentage is calculated by checklist items marked **Done**, not by lines of code. A
check is only Done after the implementation and regression coverage have been inspected;
execution status is tracked separately and must not be inferred from tests merely existing
in source.

## P0-1 — URL and origin parsing
- [ ] Reject malformed URL authorities (invalid host characters, malformed userinfo, ports, and IPv6).
- [ ] Ensure origin, cookie, policy, and transport code use consistent host/port interpretation.
- [ ] Add regression cases for malformed and valid authority forms.
- [ ] Run the URL and networking test suites on CI.

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
