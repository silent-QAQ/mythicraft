# Cross-window integration gaps

These are concrete follow-up tasks discovered by the Window 4 black-box suite. Window 4 does not patch the owning crates directly.

## W1-WORLD-LOAD-001

- Owner: Window 1.
- Current evidence: the production `server/` path delegates world loading to Pumpkin and now runs the bounded `mythicraft-world` map diagnostic after Pumpkin's authoritative `level.dat` preflight. The integration harness now covers valid, corrupt-region and unsupported-`DataVersion` cases; no real licensed map evidence has been recorded yet.
- Required fix: add runner-provided real-map evidence that records `DataVersion`, region/区块 summary, map hash, and corrupt-region failure results.
- Acceptance: execute the production entry with a licensed map on a runner and retain the diagnostic log/evidence artifact.

## W1-NBT-FUZZ-001

- Owner: Window 1.
- Current evidence: `crates/nbt::parse_named_root` and `crates/world::inspect_world_directory` now expose bounded NBT/Anvil parsing with size, depth, collection, compression, palette and `DataVersion` checks; the integration harness exercises the parser through a cross-window `world_load` stage.
- Required fix: add long-running fuzz/property coverage without relaxing the existing bounds.
- Acceptance: Window 4 can run the existing world security corpus without private implementation access.

## W2-YAML-BOM-001

- Owner: Window 2.
- Current evidence: `import_mythicmobs` normalizes one optional UTF-8 BOM before YAML parsing; the Pumpkin integration passes the same normalization path.
- Required fix: retain direct-import regression coverage so future parser changes do not reintroduce BOM failures.
- Acceptance: importing `fixtures/compat/mythicmobs/basic.yml` directly succeeds and retains the unknown-field diagnostic.

## W3-ACTION-PAGE-001

- Owner: Window 3.
- Current evidence: Pumpkin binds actions to the player's active page ID/version/nonce and performs permission, range, state, expiry and replay checks before execution. The generic crate-level gate remains intentionally actor-agnostic and must be scoped by its caller.
- Required fix: add a cross-actor/session regression case to the Pumpkin-backed integration stage; do not weaken the generic gate's reusable API.
- Acceptance: cross-page actions with otherwise matching version and nonce are rejected; the same request ID from different actors is scoped correctly.
