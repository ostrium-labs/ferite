# ferite monorepo

`ostrium-labs/ferite` is the canonical repository for Lithic and Strata.
The repository and site are named `ferite`. Existing `ferrite-lithic*` and
`ferrite-strata*` crate names and public APIs stay unchanged.

## Layout

| Path | Responsibility |
|---|---|
| `crates/ferrite-lithic*` | Hardware construction, simulation, RTL, verification, process and chip tooling |
| `crates/ferrite-strata*` | Tensor graph, partitioner, runtime, CPU reference backend and plugin ABI |
| `docs/adr/` | Lithic decisions, with their original numbers |
| `docs/strata/adr/` | Strata decisions, with their original numbers |
| `docs/strata/` | Strata README, details, design notes and original notice |
| `site/` | One documentation site for both halves |

The root Cargo workspace discovers all sixteen crates through `crates/*`.
One lockfile, toolchain, lint policy and CI govern both families. The Strata
dependency policy remains a CI gate alongside Lithic's cosimulation and synthesis
gates. The Apache-2.0 licence applies to both; original notices are retained.

## The ouroboros loop

1. **Strata feeds Lithic:** real inference subgraphs, dtype/shape constraints and
   CPU-reference outputs define candidate hardware workloads and correctness checks.
2. **Lithic verifies hardware:** build a circuit, compare simulation with emitted
   RTL, and measure synthesis area and cycle behaviour.
3. **Lithic feeds Strata:** a future hardware backend exposes measured supported
   patterns and resource limits through Strata's capability contract.
4. **Strata closes the loop:** placement, compile declines and host fallback reveal
   the next unsupported or expensive workload to improve in Lithic.

This is a co-design loop, **not a circular Cargo dependency**. The tensor graph
and the circuit IR remain distinct. A future adapter should depend on both public
APIs without making either core depend on the other.

## Implemented versus planned

Both crate families are present in the workspace. Strata includes the graph,
CPU kernels, partitioning, host execution, and plugin ABI/wire/loader code.
The presence of plugin code is not a claim of production vendor integration.
CubeCL, PyTorch integration, a Lithic Strata backend, tensor-to-circuit lowering,
FPGA transport and bitstream deployment are not implemented here. No end-to-end
hardware acceleration or new performance result is claimed by the merge.

## Working from one root

```sh
cargo test --workspace --all-features --locked
cargo test -p ferrite-strata-runtime --locked
cargo test -p ferrite-lithic-sim --locked
cargo doc --workspace --no-deps --all-features --locked
cd site
npm ci
npm run typecheck
npm test
npm run build
```

Hardware-tool tests retain their existing explicit skip behaviour locally; CI's
dedicated hardware jobs require those tools. Recorded Lithic test/benchmark
counts are historical Lithic snapshots, not new monorepo totals.

## Migration scope

Strata source and documentation were imported without its nested `.git`, old
workspace manifest, lockfile or duplicate CI. The original sibling checkout and
its Git history are intentionally untouched as a recovery source. This is a
source consolidation, not a Git-history merge. The imported Strata revision is
`c13249544dc5b0cc7f46efb97e9d1feef7d9b2ec`; its original history remains in
`ostrium-labs/ferrite-strata`. The canonical GitHub repository was renamed to
`ostrium-labs/ferite` at the owner's request. The old Strata remote is retained
as a read-only archive once the consolidated source is pushed. Future work
belongs here.
