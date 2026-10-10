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
