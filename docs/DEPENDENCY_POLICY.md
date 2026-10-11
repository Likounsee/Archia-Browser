# Rust dependency policy

This policy applies to the `Archia-Browser` branch and the Rust workspace rooted at the repository root.

## Reproducible dependency resolution

- `Cargo.lock` is committed and must be changed in the same commit as an intentional dependency update.
- CI uses `cargo metadata --locked`, `cargo test --workspace --locked`, `cargo build --workspace --locked`, and `cargo clippy --workspace --all-targets --locked -- -D warnings`. A dependency graph that requires rewriting the lockfile fails validation.
- Do not update dependencies as an unrelated bulk change. Review the dependency's role, feature flags, transitive impact, license, maintenance status, and security advisories before changing the lockfile.

## Required checks

Run these checks after dependency changes and before merging:

```sh
cargo metadata --locked --format-version 1 --no-deps
cargo test --workspace --locked
cargo build --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo audit --deny warnings
cargo deny check licenses
```

The GitHub Actions workflow runs the build/test checks on Linux and Windows. RustSec and license audits run on Linux; Windows skipping those two audit steps is intentional because the audit results do not depend on the target OS.

## Security advisory handling

- Treat any RustSec advisory reported by `cargo audit --deny warnings` as a release blocker until reviewed.
- Prefer an upstream patched release compatible with the project's supported Rust toolchain. Update the lockfile deliberately and rerun the full checks.
- If no safe update is available, do not silently suppress the advisory. Record the advisory ID, affected dependency and versions, reachable attack surface, mitigation, owner, and a review/expiry date in a dedicated issue or security decision. Critical/high advisories require explicit maintainer acceptance before any release; otherwise the release remains blocked.
- Never claim that a dependency is safe merely because the build succeeds.

## License handling

- `cargo deny check licenses` is required in CI. Review license changes in both direct and transitive dependencies before updating.
- Do not add blanket license allowances to make CI pass. Any exception must identify the dependency, license, rationale, scope, and approval in a reviewed change.

## Update cadence and evidence

- Review dependency advisories before each release and whenever a security update is proposed.
- Keep the lockfile, audit output/CI run, and rationale for exceptions traceable to the commit being validated.
- A green CI run proves only the checks that actually executed for its SHA. It does not replace a review of the application's use of a dependency or the broader P0 security audit.
