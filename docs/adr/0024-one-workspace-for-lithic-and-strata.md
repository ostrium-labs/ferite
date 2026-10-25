# One workspace for Lithic and Strata

Status: Accepted

## Decision

Use `ferite` as the canonical repository and site name. Import the Strata
crates into the existing flat Cargo workspace, preserve public crate names and
keep Strata's ADR numbering under `docs/strata/adr/`. Publish both halves through
the existing site and retain Strata's dependency-policy CI gate.

## Rationale

Workload-driven hardware and capability-driven runtime placement are opposite
directions of one feedback loop. Shared changes should be reviewable together,
without cross-repository version drift or a second documentation deployment.

## Consequences

One lockfile, toolchain, CI and documentation navigation cover both families.
Their IRs and responsibilities remain separate. The merge does not implement
the hardware adapter or introduce circular dependencies. Historical repository
names in earlier ADRs describe their original context; this decision supersedes
the two-repository layout. See `../monorepo.md` for migration scope.
