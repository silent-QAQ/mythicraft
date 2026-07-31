# ADR 0002：采用 Pumpkin 作为 Mythicraft 服务端运行时底座

## 状态

已接受

## 背景

Mythicraft 的目标不是重复实现一个通用 Minecraft 服务端，而是为 RPG 服务器提供固定版本、既有地图优先、客户端 Mod 增强的运行时。Pumpkin 已具备 Java 网络监听、Login → Configuration → Play 生命周期、Anvil 世界加载、区块/实体/tick、原版配置和插件生命周期。继续在 Mythicraft 中并行维护另一套协议、世界和连接主循环，会产生两套权威状态源和重复的版本兼容成本。

项目已获得 Pumpkin 代码的直接使用授权。

## 决策

1. `server/` 使用 Pumpkin workspace 的 `pumpkin`、`pumpkin-config` 和 `pumpkin-data` 作为路径依赖，`PumpkinServer` 是真实服务端启动入口。
2. Pumpkin 负责 Minecraft 网络、协议状态、世界加载、玩家/实体生命周期、tick 和原版插件生命周期。
3. Mythicraft RPG、经济、权限和客户端 payload 核心直接编译进 Pumpkin 的 `Server`、ticker、玩家和网络路径；Paper/MythicMobs 配置迁移、持久化和观测仍由 Mythicraft crate 提供，经过明确的核心接口进入运行时。
4. 现有 `crates/protocol`、`crates/session`、`crates/nbt`、`crates/world` 保留为离线检查、契约模型、fixture 和兼容验证；不再作为生产网络/世界主循环的权威实现。`Pumpkin-master/pumpkin/src/mythicraft.rs` 是核心 RPG 入口。
5. 外层 Mythicraft workspace 明确排除 `Pumpkin-master`，避免 Cargo 把 Pumpkin 的 workspace 依赖错误解析到 Mythicraft 根；`server/` 仍通过路径依赖引用它。

## 许可证影响

Pumpkin 为 GPL-3.0。发布包含 `mythicraft-server` 的发行物必须保留版权和许可证通知，并履行适用的源代码提供等义务。Pumpkin 之外的 Mythicraft crate 继续按其自身许可证管理；第三方插件、客户端 Mod、地图和资源不因本 ADR 自动获得分发授权。

## 后果

### 正面

- 直接获得成熟的真实 Login/Configuration/Play、世界和 tick 路径。
- 避免 Mythicraft 自有协议/世界实现与 Pumpkin 发生双重维护和状态分叉。
- 可以把开发资源集中到 RPG、配置兼容和客户端体验。

### 代价

- Pumpkin 版本、协议和 workspace 变化成为底座升级约束。
- 需要维护 MythicraftCore 与 Pumpkin 内部类型的耦合，以及 Pumpkin 许可证发行流程。
- Mythicraft 原有基础 crate 的生产职责需要逐步收缩为 adapter、离线工具和测试契约。

## 验证计划

- 轻量清单验证：`cargo metadata --no-deps --format-version 1`、`cargo fmt --all --check`。
- 真实构建和客户端连接验证：按项目约定提交 GitHub 仓库 `https://github.com/silent-QAQ/mythicraft`，由 GitHub Actions 执行；本地工作流不重复构建已由用户实测可构建的 Pumpkin Rust 项目。
