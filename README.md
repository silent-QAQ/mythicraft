# Mythicraft

Mythicraft 是一个面向 RPG 服务器的独立 Minecraft 服务端核心设计项目，目标是以 Rust 构建高性能、固定地图优先的服务端运行时，并通过客户端 Mod 提供 RPG 专用 UI、资源和音效能力。

当前仓库已经进入 Pumpkin 原生核心集成阶段：服务端入口、MythicraftCore、配置导入器、RPG IR、客户端 payload 契约和持久化基础设施已在代码中；完整的实体技能闭环、真实客户端验证和发布构建仍按下方计划推进。

权威文档如下：

- [可行性分析](docs/01-feasibility-analysis.md)：目标拆解、技术边界、兼容策略、风险和第一版验收标准。
- [AI 开发计划](docs/02-ai-development-plan.md)：面向 AI 编码代理的架构、里程碑、任务模板、测试门禁和交付节奏。
- [最终总体开发计划](docs/03-final-development-plan.md)：结合 Pumpkin、Steel、MythicMobs、LuckPerms、VaultUnlocked、ArcartX 和 DragonCore 本地参考资料后的正式路线。
- [AI 总计划](docs/04-ai-master-plan.md)：四个并行 AI 对话窗口共享的工程协议、契约、依赖和质量门禁。
- [AI 窗口 1：基础运行时](docs/ai/01-foundation-runtime.md)
- [AI 窗口 2：RPG 与兼容层](docs/ai/02-rpg-compatibility.md)
- [AI 窗口 3：客户端 UI 与协议](docs/ai/03-client-ui-protocol.md)
- [AI 窗口 4：集成、测试与发布](docs/ai/04-integration-testing-release.md)

## 已实现基础设施

- `Pumpkin-master/pumpkin/src/mythicraft.rs`：RPG、经济、权限和客户端 payload 的第一层原生核心入口，直接挂入 Pumpkin 的 Server、玩家、tick 和 Java payload 路径。
- `crates/arcartx` 与 Pumpkin 原生接入：扫描 `plugins/ArcartX/ui` 等目录，保留 ArcartX UI/tooltip 原始模型，HUD 自动打开；UI 动作支持显式服务端命令桥或客户端 `ui_run`，其他页面可用 `/mythicraft ui <页面ID>` 打开。
- `server`：使用 Pumpkin 负责真实 Minecraft Java 生命周期，支持 `--root`、既有 `level.dat` 预检、MythicMobs 目录扫描和优雅停止。
- `crates/persistence`：版本化玩家存档、校验和、原子写入、跨实例 OS 锁、强杀恢复、备份恢复、schema 迁移、配置 last-known-good 和幂等经济审计。
- `crates/observability`：JSONL 集成阶段结果、结构化日志初始化和 p50/p95/p99 性能报告。
- `tools`：fixture 校验、发布内容扫描、存档检查/备份/恢复和性能门禁命令。
- `tests/integration-harness`：已接入协议/会话、版本矩阵、RPG 导入与战斗、权限、经济、客户端能力/UI action 和持久化的机器可读闭环；Anvil 世界读取仍明确标记为跳过。

常用门禁：

```text
cargo test -p mythicraft-persistence -p mythicraft-observability -p mythicraft-integration-harness
cargo run -p mythicraft-tools -- fixture-verify fixtures
cargo run -p mythicraft-tools -- release-scan crates/persistence crates/observability tools tests fixtures docs .github
```

## 一句话结论

可行，但应兼容 Paper/MythicMobs/Vault 的配置与语义子集，而不是兼容 Paper 插件 Java 二进制或 Bukkit/Paper API；第一版锁定一个 Minecraft 协议版本，读取既有 Anvil 地图，暂不实现地形生成。
