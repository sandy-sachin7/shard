## Summary

<!-- One or two sentences describing what this change does and why. -->

## Related Issue

Closes #...

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` passes
- [ ] `cargo test --workspace --locked` passes
- [ ] `cargo audit` passes
- [ ] Zero non-test `.unwrap()` / `.expect()` introduced under `src/`
- [ ] Added unit/integration tests covering the change
- [ ] Updated `README.md` / `docs/src/` if CLI flags or user-facing behavior changed

## Type of Change

- [ ] Bug fix
- [ ] New feature
- [ ] Security hardening
- [ ] Performance improvement
- [ ] Documentation
- [ ] Infrastructure (CI, packaging, config)
