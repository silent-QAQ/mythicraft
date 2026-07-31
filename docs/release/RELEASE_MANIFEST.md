# Release manifest schema v1

The manifest may live inside or outside the staging directory. Paths are always relative to the staging root and may not contain absolute components or `..`.

```json
{
  "schema_version": 1,
  "package_version": "0.1.0-preview.1",
  "target": "x86_64-unknown-linux-gnu",
  "files": [
    {
      "path": "bin/mythicraft-server",
      "sha256": "<64 lowercase hexadecimal characters>",
      "role": "server_binary",
      "source": "Mythicraft release build",
      "license_status": "repository license",
      "redistributable": true
    }
  ]
}
```

Required roles are `server_binary`, `example_config`, `map_checker`, `config_migrator`, `client_mod`, `resource_manifest`, `compatibility_matrix`, `runbook`, and `known_limitations`. Additional declared files and roles are allowed when their provenance is complete.

The validator rejects missing roles, duplicate paths, undeclared files, symlinks, hash mismatches, empty provenance, non-redistributable entries, Java jars/classes, and unreviewed `level.dat` files.
