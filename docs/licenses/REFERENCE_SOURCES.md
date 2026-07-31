# Reference sources and redistribution status

The directories below are local research inputs and source dependencies. Pumpkin is used as an authorized server-core dependency; release-package contents remain subject to the per-file release scan, dependency license check, and license notices.

| Local input | Recorded version | Observed license/status | Permitted use in this repository |
|---|---:|---|---|
| Pumpkin | `0.1.0-dev+26.2` | GPL-3.0 | Authorized direct use as the `mythicraft-server` runtime base; preserve GPL notices and source obligations. |
| Mythicraft RPG/compat/client-services/economy/permission crates | repository `0.1.0` | MIT under the outer workspace metadata | Directly linked into Pumpkin's native `MythicraftCore`; retain repository notices and do not describe these crates as a separate plugin ABI. |
| SteelMC | `0.14.0+mc26.2` | AGPL-3.0-or-later | Architectural comparison only; do not copy code or generated data. |
| MythicMobs | `5.13.0` | Redistribution not established from the provided extracted files | Observe configuration semantics; never package classes, jars, or assets. |
| LuckPerms | local `master` snapshot | License file present; release reuse requires separate review | Observe permission semantics; never package plugin binaries. |
| VaultUnlocked | `2.20.2` | License file present; release reuse requires separate review | Observe economy API semantics; never package plugin binaries. |
| ArcartX | `2.5.36` | Redistribution not established | Feature inventory only; never copy private protocols, classes, or assets. |
| DragonCore | `2.6.2.9` | Redistribution not established | Feature inventory only; never copy classes, jars, assets, or update mechanisms. |

All committed fixtures in `fixtures/manifest.json` are project-authored synthetic inputs. A future non-synthetic fixture requires an explicit source URL or acquisition record, immutable hash, upstream version, license decision, and reviewer approval. The Pumpkin path dependency and the direct Mythicraft crate links in `Pumpkin-master/pumpkin/Cargo.toml` are intentional source dependencies and are tracked as part of the native core integration. A distribution containing Pumpkin must satisfy GPL-3.0 notice and corresponding-source obligations; the license of a combined distribution must be reviewed before release.

## CI license evidence

`.github/workflows/ci.yml` runs `cargo deny --config .github/deny-ci.toml --manifest-path <manifest> check advisories bans licenses sources` for both `Cargo.toml` and `Pumpkin-master/Cargo.toml`, covering advisories, bans, licenses, and sources. It uses `.github/deny-ci.toml`, whose GPL-3.0 entries exist only to account for the authorized direct Pumpkin runtime dependency. A successful check is evidence for the checked commit and dependency resolution; it is not a redistribution decision for MythicMobs, LuckPerms, VaultUnlocked, ArcartX, DragonCore, or any other local reference tree.

For a release candidate, retain the dependency-check output, the exact commit SHA, the resolved lockfile inputs, the Pumpkin source/license notice and corresponding-source record, and the per-file `source`, `license_status`, and `redistributable` fields required by `docs/release/RELEASE_MANIFEST.md`. `release-scan` detects forbidden Java binaries, symlinks, and unreviewed `level.dat`; it does not replace human license review.
