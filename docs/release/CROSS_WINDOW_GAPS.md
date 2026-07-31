# Cross-window integration gaps

These are concrete follow-up tasks discovered by the Window 4 black-box suite. Window 4 does not patch the owning crates directly.

## W1-WORLD-LOAD-001

- Owner: Window 1.
- Current evidence: the legacy `crates/world` black-box `world_load` stage is machine-readably marked `skipped`; the production `server/` path now delegates world loading to Pumpkin, but no Mythicraft map-diagnostic bridge or real-map evidence has been recorded yet.
- Required fix: either expose a bounded Pumpkin world/map-summary adapter or explicitly retire the legacy stage in favor of a Pumpkin-backed black-box stage with `DataVersion` diagnostics, map hash, and corrupt-region failure results.
- Acceptance: replace the skipped stage with a real valid/corrupt/unsupported world fixture test against the selected production path.

## W1-NBT-FUZZ-001

- Owner: Window 1.
- Current evidence: `crates/nbt` currently exposes limits but no parser entry point.
- Required fix: expose bounded NBT/Anvil parsing for corrupt compression, recursion depth, large lists, palette validation, and unknown `DataVersion`.
- Acceptance: Window 4 can run the existing world security corpus without private implementation access.

## W2-YAML-BOM-001

- Owner: Window 2.
- Current evidence: `import_mythicmobs` rejects the shared UTF-8-BOM fixture unless the integration adapter strips the BOM first.
- Required fix: normalize one optional UTF-8 BOM before YAML parsing without changing diagnostic line/column semantics.
- Acceptance: importing `fixtures/compat/mythicmobs/basic.yml` directly succeeds and retains the unknown-field diagnostic.

## W3-ACTION-PAGE-001

- Owner: Window 3.
- Current evidence: `UiActionContext` binds page version and nonce but has no expected page ID or actor/session identity.
- Required fix: bind action authorization to active page ID and actor/session ID before replay/rate-limit recording.
- Acceptance: cross-page actions with otherwise matching version and nonce are rejected; the same request ID from different actors is scoped correctly.
