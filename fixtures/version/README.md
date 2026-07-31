# Version fixtures

`26.2-draft.json` pins the server-side values derived from the local Pumpkin and Steel reference snapshots:

- Minecraft Java `26.2`
- protocol `776`
- world `DataVersion` `4903`
- SHA-256 of Pumpkin's local `26_2_synced_registries.json`

The fixture intentionally remains invalid while the client loader and loader version are `pending`. Window 3 must freeze those fields before this becomes a startup-accepted matrix. Registry data itself is not copied into Mythicraft.
