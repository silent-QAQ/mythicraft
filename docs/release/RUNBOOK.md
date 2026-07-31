# Mythicraft technical preview runbook

## Prerequisites

- Rust `1.96.0` with `rustfmt` and `clippy`.
- A version matrix whose Minecraft version, protocol, `DataVersion`, and client loader are no longer `TBD`.
- A checked, redistributable fixed world and matching server/client configuration.

## ArcartX UI 配置

将兼容的 ArcartX 页面文件放到服务端根目录的
`plugins/ArcartX/ui/`（也支持 `plugins/ArcartX/tooltip/`、`plugins/ArcartX/UI/`、
`config/arcartx/{ui,tooltip}`、`config/arcartx` 和 `arcartx/{ui,tooltip}`、`arcartx`）。核心启动时递归读取
`.yml`、`.yaml`、`.json`；`ui.isHud: true` 且
`defaultOpen` 不是 `false` 的页面会在客户端能力协商后自动打开，其他页面使用：

```text
/mythicraft ui <页面ID>
```

ArcartX UI/tooltip 配置可放在 `plugins/ArcartX/ui`、`plugins/ArcartX/tooltip`，或对应的
`config/arcartx/{ui,tooltip}`、`arcartx/{ui,tooltip}` 目录；文件名默认作为页面 ID，支持
中文文件名。配置变更后重启核心生效，当前版本不会热重载 ArcartX 文件。

当前页面模型、控件、动作、权限和资源引用会保留并映射到 Mythicraft 原生 UI 协议。
Aria 脚本、旧 `arcartx:main` 加密封包、CRC/签名资源同步和非 UI 配置目录尚未声明
兼容；启动日志会报告未知字段、类型错误和重复页面。

## 本地与 GitHub 构建分工

本地交付工作流只执行不产生 Rust 编译产物的轻量检查。明确约束：本地不得执行 `cargo build`、`cargo check`、`cargo test` 或 `cargo clippy`；这些命令只能由 GitHub Actions 运行并留下可追溯的 run evidence。

```text
cargo metadata --no-deps --format-version 1
cargo fmt --all --check
cargo fmt --manifest-path Pumpkin-master/Cargo.toml --all --check
```

完整的外层 workspace、`mythicraft-server` 及其 Pumpkin path dependency 构建，Pumpkin 独立 workspace 的构建/测试，依赖许可证检查，以及 artifact 完整性 smoke test，由 [Mythicraft GitHub 仓库](https://github.com/silent-QAQ/mythicraft) 的 `.github/workflows/ci.yml` 执行。本地未执行这些 Rust build/check/test 命令；格式或 metadata 通过不等于构建通过。

### GitHub Actions 命令与覆盖范围

CI 会执行以下关键命令；它们是 CI 命令，不是本地验收命令：

```text
cargo build --manifest-path Cargo.toml --workspace --all-targets
cargo check --manifest-path Cargo.toml --workspace --all-targets
cargo build --manifest-path Cargo.toml --package mythicraft-server --release
cargo clippy --manifest-path Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path Cargo.toml --workspace --all-targets
cargo fmt --manifest-path Pumpkin-master/Cargo.toml --all --check
cargo build --manifest-path Pumpkin-master/Cargo.toml --workspace --all-targets
cargo check --manifest-path Pumpkin-master/Cargo.toml --workspace --all-targets
cargo test --manifest-path Pumpkin-master/Cargo.toml --workspace --all-targets
cargo run --manifest-path Cargo.toml -p mythicraft-tools -- fixture-verify fixtures
cargo run --manifest-path Cargo.toml -p mythicraft-tools -- release-scan crates tools tests fixtures docs .github
cargo deny --config .github/deny-ci.toml --manifest-path Cargo.toml check advisories bans licenses sources
cargo deny --config .github/deny-ci.toml --manifest-path Pumpkin-master/Cargo.toml check advisories bans licenses sources
cargo run --manifest-path Cargo.toml -p mythicraft-tools -- release-scan dist/ci-artifact
```

`Cargo.toml` 把 `Pumpkin-master` 排除在外层 workspace 之外，所以外层 `--workspace` 不代表 Pumpkin workspace 全量门禁。`server/Cargo.toml` 的 Pumpkin path dependency 会在构建 `mythicraft-server` 时被解析和编译；CI 另外对 `Pumpkin-master/Cargo.toml` 执行独立 workspace build/test，避免把这两个事实混为一谈。

### Artifact 与真实客户端 smoke

CI artifact 是诊断构建产物，不是可发布预览包。它包含 release server binary 和 `SHA256SUMS`，随后在独立 job 中解包、校验 SHA-256 和可执行权限。该 job 不启动真实 Minecraft 客户端，也不能替代实机证据。

真实 smoke 只在手动 dispatch 且选择 `run_real_smoke=true` 时运行，并要求标签为 `self-hosted, linux, mythicraft-real-smoke` 的 runner 提供可执行仓库变量 `MYTHICRAFT_REAL_SMOKE_SCRIPT`。脚本契约为：接收 `<server-binary> <runtime-root> <report-json>` 三个参数，负责启动服务端、连接固定版本真实客户端并执行登录/Configuration/Play、固定地图加载、关键 payload/UI 动作和优雅退出；必须写出非空 JSON 报告，失败时返回非零状态。没有该 runner、脚本、固定地图和客户端包时，smoke 是未执行状态，不得写成通过。

真实 smoke 至少需要上传：GitHub run URL、commit SHA、Rust/toolchain/target、server binary SHA-256、客户端版本与 loader、地图和配置 hash、服务端日志、客户端日志、机器可读 JSON 报告，以及失败时的退出码和原始证据。

### 未覆盖风险

- 当前外层 `Cargo.lock` 不包含被排除的 Pumpkin workspace 的完整包集合；CI 没有使用 `--locked`，因此首次解析可能更新临时 runner 上的 lockfile。发布前必须保存解析后的 lockfile 和依赖检查输出，并单独决定是否冻结该依赖边界。
- CI artifact job 只验证 server binary 的归档和 hash，没有运行 `release-manifest-verify`；正式发布仍必须提供全部必需角色并对最终 staging 执行 manifest 校验。
- 真实客户端 smoke 使用 runner 提供的外部脚本、固定地图和客户端安装，不是仓库内可复现的测试 harness；这些输入缺失时，真实 smoke 保持未执行状态。
- CI 许可证 allow-list 包含 GPL-3.0 以覆盖 Pumpkin 直接依赖，但 cargo-deny 结果不能替代逐文件来源、通知文本和对应源码义务审查。

## GitHub quality gate

```text
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mythicraft-tools -- fixture-verify fixtures
cargo run -p mythicraft-tools -- release-scan <staging-directory>
cargo run -p mythicraft-tools -- release-manifest-verify <staging-directory> <release-manifest.json>
cargo deny check advisories bans licenses sources
```

上面的简写用于人工复核；实际 GitHub 命令以 `.github/workflows/ci.yml` 为准，并使用 `.github/deny-ci.toml`。许可证配置允许 GPL-3.0 是因为 `mythicraft-server` 直接链接已授权的 Pumpkin 核心；这不是对其他 GPL/AGPL 参考项目或第三方二进制的再分发授权。

The cross-crate vertical slice writes one JSONL result per required stage. A failed operation is written with `status: failed` before the test returns an error; unavailable owning-window functionality is written as `status: skipped`, never as a synthetic pass.

## Save operations

Inspect a player save:

```text
cargo run -p mythicraft-tools -- save-inspect <save-root> <player-id>
```

Create an operator-labelled backup:

```text
cargo run -p mythicraft-tools -- save-backup <save-root> <player-id> <label>
```

Restore a backup as a new revision:

```text
cargo run -p mythicraft-tools -- save-restore <save-root> <backup-file>
```

Never edit live save JSON. Stop player mutations, create a backup, restore through the command, then verify the revision and audit record.

All save, backup, restore, audit, and economy operations take an OS-level exclusive lock on `<save-root>/persistence.lock` in addition to the in-process mutex. Independent server/tool instances therefore serialize access to one save root. Do not place the save root on a network filesystem unless its advisory-lock and atomic-rename behavior has been independently verified.

## Performance report

Prepare JSON containing `metadata`, `tick_samples_ms`, `memory_peak_bytes`, `network_bytes_per_second`, and `blocking_tick_io_events`, then run:

```text
cargo run -p mythicraft-tools -- perf-report <input.json> <output.json>
```

The command fails unless tick p95 is at most 35 ms, p99 is at most 50 ms, and blocking tick IO events equal zero. Metadata must identify scenario, machine, OS, Rust profile, target, Minecraft version, map/config hashes, player/entity counts, skill rate, and duration.

After producing reports, verify the complete suite:

```text
cargo run -p mythicraft-tools -- perf-suite-verify <report-directory>
```

The required scenarios are static player movement, entity-dense load, high-frequency skills, multiplayer visibility broadcast, bulk chunk send, economy concurrency, and high-frequency UI/audio events.

## Crash recovery

Player saves use a checksummed primary file plus `.tmp` and `.bak` candidates. Startup selects the highest valid revision and promotes it to the primary path. Invalid checksums, unknown schemas, negative balances, stale revisions, and path-like identifiers fail closed.

The integration suite launches a helper process that interrupts a save after the primary file has been renamed to backup, then calls `abort`. Reopening the store must release the abandoned OS lock and promote the valid temporary revision. This verifies process termination rather than only an in-process injected error.

Economy operations persist a transaction marker, player revision, immutable per-transaction audit record, and committed marker. Retries return a duplicate outcome without applying the amount twice.

## Troubleshooting

- `checksum mismatch`: preserve all three save candidates and restore a known backup; do not delete evidence.
- `revision conflict`: reload current player state and retry the domain operation against the new revision.
- process appears stuck on `persistence.lock`: confirm another server or recovery tool is not using the same save root; never delete a live lock file to bypass serialization.
- `unsupported player schema`: retain the file and add an explicit migration plus fixture.
- `fixture hash mismatch`: verify whether the input changed intentionally; update `fixtures/CHANGELOG.md` before accepting a new hash.
- `tick latency gate failed`: keep the generated report, identify blocking IO or overload, and rerun with identical metadata.
- `performance suite is missing scenario`: run the missing scenario with the same build, map, and configuration hashes.
- `release file is not declared`: regenerate the release manifest; do not silently remove or ignore the file.
