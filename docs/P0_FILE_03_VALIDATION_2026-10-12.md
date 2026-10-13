# P0-FILE-03 — validation checkpoint (2026-10-12)

> **Historical checkpoint — not the current branch assessment.** This report records the source and evidence available on 2026-10-12 for SHA `03905fbb893d36f76f379c6987ce0f521e66c3c9`. Later commits changed the local-file transport and its confinement strategy. Use `ROADMAP_P0_P3.md` and CI evidence for the exact candidate SHA for current status; do not apply this report's pathname-based finding to later code without re-inspecting that code.

## Branch and reviewed head

- Working branch: `Archia-Browser` (confirmed present in GitHub).
- Latest commit found in the repository commit search at review time: [`03905fbb893d36f76f379c6987ce0f521e66c3c9`](https://github.com/Likounsee/Archia-Browser/commit/03905fbb893d36f76f379c6987ce0f521e66c3c9) — `Scope local browser navigation to selected file directory`, committed 2026-10-12 08:41:41 +02:00.
- Its parent-chain predecessors include [`492202c32985beb53e9da1f43e7f85336caf6fba`](https://github.com/Likounsee/Archia-Browser/commit/492202c32985beb53e9da1f43e7f85336caf6fba) — URL-scoped transport constructor. The current branch also contains the local-root and symlink hardening commits documented in `ROADMAP_P0_P3.md`.

## Code reviewed

- `Archia-Browser/src/net/transport.rs`: `LocalFileTransport` denies access when no allowed roots are configured, rejects non-local authorities and non-GET/HEAD requests, decodes the path once, rejects NUL, checks path components, canonicalizes the candidate and roots, checks containment, opens the leaf with platform-specific no-follow/reparse-point flags, checks the opened handle metadata, and bounds reads by `max_file_size`.
- `Archia-Browser/src/net/loader.rs`: tests cover local-document redirects to network URLs and network redirects to local files; the local navigation entry point uses `LocalFileTransport::for_file_url`.
- `.github/workflows/ci.yml`: push builds on `Archia-Browser`; Linux and Windows run locked metadata, rustfmt, workspace tests, build, and Clippy; Linux additionally runs RustSec and license audits.
- `.github/workflows/fuzz.yml`: scheduled/push fuzz workflow covers HTTP framing, URL, HTML/DOM, CSS tokenization/stylesheet/selectors, and cookies.

## Blocking security finding — do not close P0-FILE-03

The transport still performs pathname-based checks (`symlink_metadata`, `canonicalize`, then `open`). The final-component no-follow flag does not pin parent directories. A concurrent rename/replacement of a checked parent directory can therefore invalidate the containment decision between check and open (TOCTOU). The source comments already acknowledge this limitation. Windows reparse-point handling also needs validation against parent-component races; a pathname check is not a handle-relative confinement guarantee.

A safe follow-up should open an approved root directory once and walk relative components using directory handles and no-follow semantics (or an equivalent native primitive with equivalent guarantees). If the implementation cannot provide that guarantee on an OS, deny local-file reads there until it can. Do not weaken checks or claim race resistance based on ordinary symlink tests.

## Validation evidence available at this checkpoint

- GitHub's combined-status query for the reviewed SHA returned no status entries.
- The available commit-workflow-runs query returned no runs (that connector query is limited to pull-request-triggered runs). This is **not** evidence that CI passed or failed.
- The repository workflow definition is present, but no successful Linux/Windows run for the reviewed SHA could be verified through the available results.
- No test, build, Clippy, audit, or platform-race test was executed in this session. Do not treat this checkpoint as validation.

## Decision

`P0-FILE-03` remains **PARTIAL / NOT VALIDATED**. No P0 completion percentage should be increased on the basis of this checkpoint. Required next steps: implement handle-relative confinement per supported OS (fail closed elsewhere), add race/adversarial tests, then obtain successful Linux and Windows CI for the resulting exact SHA and update the main roadmap with those links.
