# declmig-runner.rs repository seed

This directory is the complete seed for the future `declarative-migrations/declmig-runner.rs` repository tracked by `declmig-monorepo#6` and `declmig-interfaces#19`.

The runner is a **privileged execution plane**, not another CRUD/API service. It owns target-database reachability, execution leases/fencing, resumable execution and audit emission. Authorization, plan creation, approvals, billing and ordinary admin CRUD remain in the existing API/admin servers.

## Safety boundary implemented now

- No HTTP listener or public ingress.
- Bounded 64 KiB admission envelope.
- `serde(deny_unknown_fields)` rejects credentials, raw DSNs and arbitrary command fields structurally.
- Canonical identifiers and a lowercase SHA-256 plan digest are validated before any effect can start.
- Non-zero fencing token is mandatory.
- Timeout, retry and concurrency policy are bounded.
- Admission is deterministic and explicitly reports `side_effects_started=false`.

The current executable is a local/private admission harness over stdin. It deliberately **does not execute SQL yet**. Production queue transport, secret resolution, Postgres/Cockroach execution, lease renewal and recovery should land only after the peer TypeSpec + JSON Schema runner contract in `declmig-interfaces#19` is green.

## Verify

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

When the destination GitHub repository exists, copy this tree verbatim to its root, generate `Cargo.lock`, then add it to `declmig-monorepo` as `apps/declmig-runner.rs` submodule.
