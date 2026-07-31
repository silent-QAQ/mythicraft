# Technical preview package contract

## 当前交付状态

- GitHub Actions 已配置外层 workspace、`mythicraft-server` 的 Pumpkin path dependency、Pumpkin 独立 workspace、许可证和 artifact 门禁；在对应 commit 的 Actions run 成功前，不视为构建通过。
- artifact integrity smoke 已配置，但它只证明归档、SHA-256 和可执行文件完整性，不证明真实 Minecraft 客户端可连接。
- 真实客户端/实机 smoke 仅由专用 self-hosted runner 手动启用；当前文档不宣称该测试已运行或通过。
- 发布包仍受下方必需角色、逐文件许可证/来源信息和 `release-manifest-verify` 门禁约束。

The preview package is blocked until Windows 1-3 provide a runnable server, frozen version matrix, client Mod, resource manifest, map checker, config migrator, and compatibility outputs.

When those inputs exist, staging must contain only:

- Mythicraft server binaries and their license notices.
- Example configuration with no credentials or production player data.
- Map checker and configuration migration tools.
- Matching client Mod and resource manifest whose redistribution rights are recorded.
- Compatibility matrix, backup/restore commands, troubleshooting guide, and known limitations.

It must not contain third-party plugin jars/classes, Mojang assets, local reference-project trees, unlicensed resources, raw production saves, audit data, tokens, or private keys. Run `release-scan` against the final staging directory before creating an archive.

Every staged file must also be listed in a schema-v1 release manifest with its relative path, SHA-256, package role, source, license status, and redistribution decision. Validate it with `release-manifest-verify`; undeclared files, symlinks, missing required roles, hash mismatches, and non-redistributable entries fail the release.

`dist/ci-artifact` 不是上述发布包：它没有伪造缺失的 client mod、resource manifest、map checker 或 config migrator 角色。只有真实输入齐全、每个文件有许可证和来源证据、并通过最终 staging 的 `release-scan` 与 `release-manifest-verify` 后，才能进入发布流程。
