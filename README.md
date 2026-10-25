# ferite

Rust tools for verified hardware and vendor-neutral accelerator execution.

[![CI](https://github.com/ostrium-labs/ferite/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/ostrium-labs/ferite/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

[Website](https://ferrite.ostriumlabs.org) · [Architecture](docs/monorepo.md) · [Documentation](site/README.md)

## Two toolchains, one workspace

| | Purpose |
|---|---|
| **Lithic** | Build circuits in Rust, simulate clock-by-clock, emit Verilog, and check equivalence and synthesis. |
| **Strata** | Partition inference graphs across backends, compile subgraphs, and execute with CPU fallback. |

Sixteen crates share one Cargo workspace, lockfile, CI and documentation site.
Existing `ferrite-lithic*` and `ferrite-strata*` crate names stay unchanged.

The intended loop is **Strata workloads → Lithic hardware → Strata capabilities**.
Both toolchains exist today; the hardware adapter connecting them is **not implemented**.

## Build and test

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml).

```sh
cargo build --workspace --locked
cargo test --workspace --all-features --locked
cargo doc --workspace --no-deps --all-features --locked
```

Verilator, Icarus Verilog and Yosys enable additional hardware checks. Local tests
report skips when tools are absent; dedicated CI jobs require them.

## Documentation

- [Workspace layout, feedback loop and current limits](docs/monorepo.md)
- [Lithic implementation guide and examples](docs/lithic-guide.md)
- [Strata runtime guide](docs/strata/README.md)
- [Design decisions](docs/adr/) · [Strata decisions](docs/strata/adr/)
- [Benchmarks](docs/benchmarks.md) · [Detailed implementation history](DETAILS.md)
- [Build the documentation site](site/README.md)

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE) and the retained [Strata notice](docs/strata/NOTICE).
