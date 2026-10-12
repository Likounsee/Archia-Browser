# P0-FILE-03 — handle-relative confinement design

Status: implementation requirement, not a claim that the implementation exists.

## Security invariant

For every successful `file:` read, the opened file object must be proven to be reachable beneath the exact authorized directory handle used for the request. A pathname that was canonicalized earlier is not proof: any ancestor can be renamed or replaced between validation and open.

## Required implementation shape

1. Parse and decode the URL exactly once. Reject NUL, malformed escapes, UNC/network-share forms, drive-relative paths, and any path form whose platform interpretation is ambiguous.
2. Resolve the selected file's parent directory at the point of user authorization. Open and retain a directory handle for that root; do not repeatedly re-resolve the root from its pathname during a request.
3. Derive a relative path beneath that root and reject parent traversal and platform-specific alternate stream/device syntax where relevant.
4. Walk each intermediate component relative to the current directory handle, requiring a directory and refusing symlinks/reparse points. Never fall back to a pathname-based open after a handle-relative operation fails.
5. Open the leaf relative to the final parent handle without following links. Validate the opened handle's object type and, where possible, its final identity/containment using handle-derived information.
6. Read only from that already-validated handle and retain existing size limits. Errors, unsupported flags, race detection, and unexpected object types must fail closed.
7. On platforms where equivalent guarantees cannot be implemented and tested, return the existing connection-failure/denial error rather than using the current pathname sequence as a fallback.

## Platform acceptance criteria

### Linux

Prefer a kernel-enforced beneath-root resolution primitive such as `openat2(2)` with `RESOLVE_BENEATH` and `RESOLVE_NO_SYMLINKS` (and `RESOLVE_NO_MAGICLINKS` where applicable), or a carefully reviewed descriptor-relative component walk with equivalent guarantees. Handle unsupported-kernel errors by failing closed; do not silently retry with ordinary `open(path)`.

### Other Unix-like platforms

Use directory descriptors and `openat`-family operations with no-follow flags for every path component, verifying directory handles at each step. The exact flags differ by OS and must be validated on the CI runner; no Linux numeric flag may be reused without platform-specific verification.

### Windows

Use handle-relative NT/native operations or another reviewed approach that prevents parent replacement and rejects reparse points at every component. `FILE_FLAG_OPEN_REPARSE_POINT` on the leaf alone is insufficient. Verify the opened handle's file identity/type and ensure no component can redirect resolution outside the authorized root. If these guarantees cannot be demonstrated, deny local reads on Windows until implemented.

## Adversarial tests required

- Symlink/reparse point at each component position, including leaf and parent.
- A directory inside the authorized root renamed and replaced with a symlink/junction to an outside directory while repeated reads occur.
- Replacement of the authorized root pathname after its directory handle has been opened; access must remain confined to the original opened root.
- Leaf replacement between lookup and open, including regular-file-to-symlink and directory-to-reparse-point swaps.
- Encoded separators, dot segments, double-encoding, NUL, UNC, drive-relative paths, alternate data streams and malformed URLs.
- Verify both allowed sibling resources and denied paths outside the root.
- Network redirect to `file:` and local redirect to network remain denied by the loader/policy.
- Race tests must assert that outside sentinel contents are never returned; repeat enough iterations to exercise concurrent rename/open, and keep tests deterministic where platform APIs allow.

## Exit evidence

Do not mark P0-FILE-03 complete until:
- the implementation uses handle-relative confinement on each supported OS or explicitly denies reads on unsupported OSes;
- tests above pass on Linux and Windows, including a meaningful concurrent replacement test;
- `cargo fmt --all -- --check`, `cargo test --workspace --locked`, `cargo build --workspace --locked`, and `cargo clippy --workspace --all-targets --locked -- -D warnings` pass;
- RustSec and license audits pass on Linux;
- links to CI runs for the exact candidate SHA are recorded in `ROADMAP_P0_P3.md`.

This document is a design checklist only. It does not change the roadmap's task status or P0 completion percentage.
