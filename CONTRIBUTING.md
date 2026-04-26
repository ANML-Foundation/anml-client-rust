# Contributing to anml-client

## Development Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/Life-Savor-AI/anml-client-rust.git
   cd anml-client-rust
   git submodule update --init
   ```

2. Install Rust 1.80+ via [rustup](https://rustup.rs/).

3. Build:
   ```bash
   cargo build
   ```

4. Run tests:
   ```bash
   cargo test
   ```

## Testing

### Unit Tests

```bash
cargo test
```

### With All Features

```bash
cargo test --all-features
```

### Property-Based Tests

Property-based tests use `proptest` and are in `*_property_test.rs` files:

```bash
cargo test property_test
```

### Integration Tests

Integration tests require the `testing` feature:

```bash
cargo test --features testing --test integration
```

### Builder Validation Tests

```bash
cargo test --test builder_validation
```

## Code Style

- Run `cargo fmt` before committing.
- Run `cargo clippy` and address warnings.
- All public items must have doc comments.
- Error messages should be single-line and actionable ("expected X, got Y").

## PR Guidelines

1. Create a feature branch from `main`.
2. Keep commits focused — one logical change per commit.
3. Add tests for new functionality.
4. Ensure `cargo test --all-features` passes.
5. Ensure `cargo clippy --all-features` is clean.
6. Update `CHANGELOG.md` under `[Unreleased]`.
7. Open a PR with a clear description of the change.

## Architecture

See the `design.md` in `.kiro/specs/anml-client/` for the full architecture
overview. Key modules:

- `src/client.rs` — main `AnmlClient` entry point
- `src/disclosure/` — 7-step disclosure evaluation engine
- `src/action/` — action execution, parameter binding, validation
- `src/flow/` — multi-step flow navigation
- `src/testing/` — mock server and test utilities (feature-gated)
