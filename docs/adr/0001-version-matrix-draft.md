# ADR 0001: Draft server version matrix

- Status: proposed; blocked on Window 3 client decision
- Date: 2026-07-23

## Decision

The first server-side target is Minecraft Java `26.2`, protocol `776`, and world `DataVersion` `4903` only. The pinned registry reference hash is `3ffaca442dbbd1d9acb2b7bf2509cbd80e30dbc5349dfbad39eda7f4e6bd5a8b`.

The client loader remains `pending`. Startup validation must reject the draft matrix until Window 3 selects Fabric or NeoForge and records an exact loader version.

## Local evidence

- Pumpkin `pumpkin-util/src/version.rs` maps `26.2` to protocol `776`.
- Pumpkin `pumpkin-world/src/world_info/mod.rs` declares maximum world `DataVersion` `4903` for `26.2`.
- Steel `steel-registry/build_assets/packets.json` declares protocol `776`.
- The registry hash is calculated from the local Pumpkin reference snapshot only; the generated registry file is not copied into Mythicraft.

## Consequences

- Unknown or ranged `DataVersion` inputs are unsupported during the initial vertical slice.
- Window 3 must update `fixtures/version/26.2-draft.json` and rename it after freezing the client contract.
- A separately licensed or independently generated registry artifact is still required before chunk encoding can be implemented.
