# AI 窗口 1：Rust 核心、协议与固定地图

## 责任域

负责 Pumpkin 基础能力与 Mythicraft 领域契约的桥接：版本矩阵、Minecraft 协议/登录会话、NBT/Anvil 地图诊断、版本化 vanilla data、区块/玩家/实体快照、tick 边界和跨窗口基础契约。Pumpkin 已经提供的网络、世界和实体主循环不再在 Mythicraft crate 中重复实现。

负责目录：`crates/protocol`、`crates/session`、`crates/vanilla-data`、`crates/nbt`、`crates/world`、`crates/simulation`、`crates/entity`，以及授权 Pumpkin 源码中的 `src/mythicraft.rs`、Server/ticker/player/payload 接入点；不再通过独立插件维护 RPG 权威状态。

不负责：MythicMobs 语义、Vault/LuckPerms 业务、UI renderer、资源制作、发布压测脚本。

## 参考项目

- Pumpkin：作为已授权直接底座使用 `pumpkin-protocol`、`pumpkin-nbt`、`pumpkin-world`、`pumpkin-config` 的实现、模块边界和错误模型，并直接修改核心生命周期接入点。
- Steel：`steel-login`、`steel-protocol`、`steel-core`、`steel-registry` 的版本化和生成式数据纪律。
- 不把 Pumpkin 代码无记录复制进 Mythicraft crate；Pumpkin 运行时依赖的 GPLv3 通知和来源由 Window 4 维护。Steel 的 AGPLv3 代码/生成数据仍不得直接复制；遇到原版数据缺失必须提出阻塞，而不是凭记忆硬编码。

## 实施顺序

### F1：版本与 workspace

- 创建 Cargo workspace、Rust toolchain、错误类型、日志基础设施。
- 创建 `VersionMatrix`，绑定 Minecraft protocol、DataVersion、registry hash、客户端 Mod 版本。
- 建立 `fixtures/protocol`、`fixtures/world` 和 `fixtures/version`。

验收：版本不完整、hash 不匹配或 fixture 缺失时启动失败且有明确诊断。

### F2：Pumpkin 协议与会话适配

- 以 Pumpkin 已有 handshake/status/login/configuration/play 为事实基线，补齐 Mythicraft 的版本矩阵、能力协商和自定义 payload 入口。
- 不重新实现 VarInt、包长度、压缩、加密和 keep-alive；只在 adapter 边界增加 schema/长度/错误校验。
- 网络输入通过 Pumpkin 生命周期进入 MythicraftCore；RPG 世界状态只在核心定义的 tick 边界提交。

验收：原版客户端能连接并收到基础状态；截断包、超长包、未知状态和错误版本安全断开；重复连接不泄漏会话。

### F3：Pumpkin 世界能力与地图诊断桥

- 复用 Pumpkin 的 `level.dat`、region header、chunk 压缩数据和 chunk NBT 读取能力；为 Mythicraft map-inspector 暴露有界诊断结果。RPG 核心不得另建世界事实源。
- 解析 section palette、block states、heightmap、光照、方块实体和基础实体。
- 输出地图 DataVersion、坐标范围、region/chunk 计数、未知 tag、损坏位置和内存估计。
- 只读地图采用 `StaticWorld`；未保存区块不生成。

验收：有效、空区块、损坏压缩、超大 NBT、未知 DataVersion 均有 fixture；同一输入输出稳定摘要。

### F4：区块发送与玩家基础

- 将目标版本 registry 映射到协议区块数据。
- 实现出生点、玩家位置、基础碰撞、移动验证、传送和重连。
- 只发送允许区域和可见区块；越界行为配置化。

验收：玩家能在真实地图出生区移动、碰撞、离开/重连；palette 数值不直接作为跨版本 ID。

### F5：tick 与基础实体

- 固定 20 TPS 阶段：输入、事件、实体、碰撞、效果、输出、持久化提交。
- 实体 ID、区域分桶、可见性过滤和服务端时间。
- 异步任务通过提交队列在 tick 边界生效。

验收：固定输入 fixture 产生相同状态摘要；不存在 tick 内同步磁盘 IO；tick p95/p99 达到总体计划基线。

## 跨窗口契约

向 Window 2 提供：`EntityId`、`PlayerSnapshot`、`TickContext`、`RpgEventSink`、目标查询接口、伤害事件入口。

向 Window 3 提供：payload 编解码、玩家能力集、ServerEvent、EntitySnapshot、UI action ingress。

向 Window 4 提供：可复现启动命令、地图摘要命令、tick profiler、连接黑盒测试和状态摘要。

## 禁止事项

- 不实现地形生成、红石或完整原版 AI 作为本窗口的隐性扩展。
- 不在协议层解析 MythicMobs YAML。
- 不用 `Any`/`TypeId` 作为未来插件 ABI。
- 不用 `unwrap`/`expect` 处理网络、NBT 或地图外部输入。
- 不为了测试通过而伪造 registry、block state 或协议 ID。

## 交付格式

每次交付必须报告：修改 crate、新增 fixture、协议/地图版本、运行的测试命令、实际结果、未支持的原版行为和需要其他窗口确认的契约。
