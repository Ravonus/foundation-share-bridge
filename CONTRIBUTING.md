# Contributing to foundation-share-bridge

Thanks for your interest. This is an optional, per-user IPFS pinning companion for Foundation Archive — rescued CIDs, pinned by people who care.

## Setup

```sh
cargo build
```

Requires stable Rust (edition 2024). No nightly features in the tree.

## Code conventions

These are enforced by `clippy.toml` and the guard scripts in `scripts/lint/`. CI rejects PRs that violate them.

- **Files**: no single `.rs` file over **600 lines** (warn at 400). Split by concern.
- **Functions**: no function over **80 lines**. Decompose into typed DTOs + pure renderers + thin handlers.
- **Arguments**: no function with more than **4 parameters**. Bundle related args into a request struct or convert to `impl` methods on a service type.
- **Folders**: no directory under `src/` or `scripts/` with more than **6 direct children**. Group by concern.
- **No `unwrap`, `panic!`, `dbg!`, `todo!`, `unimplemented!`, or `unsafe`** in shipped code. `expect` is warn-level; use it only with an assert-like message explaining an invariant.
- **No lock guard held across an `await`** that calls another service (`clippy::await_holding_lock` is `deny`). Drop the guard or clone the data out first.

Broader `clippy::pedantic` and `clippy::nursery` are set to `warn`. Don't silence them without a comment explaining why.

## Before committing

Run the full lint + guard pipeline:

```sh
bash scripts/lint/run-all.sh
```

This runs: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check` (if installed), plus the custom file-size / folder-fanout / monolith guards.

To install these as pre-commit hooks:

```sh
pip install pre-commit    # one-time
pre-commit install        # one-time, in repo root
```

## PR checklist

- [ ] `bash scripts/lint/run-all.sh` passes locally.
- [ ] New or touched files respect file / function / argument limits.
- [ ] No `clippy::allow` added without a comment explaining why.
- [ ] If you touched persisted types (`BridgeConfig`, `BridgePersistentState`, relay messages), the serde attributes are unchanged or a migration is included.
- [ ] If the change affects HTML rendering, `cargo test` snapshot assertions are reviewed (`cargo insta review`).

## Licensing

By contributing you agree your contribution is licensed under Apache-2.0 (same as the project).
