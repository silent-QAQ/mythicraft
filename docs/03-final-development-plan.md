# Mythicraft 最终开发计划

> 文档状态：基于本地参考项目审阅后的正式开发基线
>
> 目标：以已授权的 Pumpkin Rust 核心为 Minecraft 协议、世界、实体和 tick 底座，构建面向 RPG 服务器的 Mythicraft 运行时，并通过客户端 Mod 提供 UI、资源、音效和模型能力。

## 1. 最终产品定义

Mythicraft 不是 Paper 的二进制兼容替代品，而是一个固定 Minecraft 版本、固定客户端 Mod 版本、固定地图优先的 RPG 服务端运行时。它兼容的是配置格式和经过定义的游戏语义，而不是运行现有 Bukkit/Paper Java 插件。

第一版产品闭环：

```text
既有 Anvil 地图
  → Rust 地图检查/导入
  → 服务端协议与区块发送
  → 玩家/实体/tick
  → MythicMobs RPG 配置子集
  → Vault/LuckPerms 兼容服务
  → 客户端 Mod UI、贴图、模型、音效
```

## 2. 参考项目审阅结论

### 2.1 Pumpkin：世界、协议和插件边界

本地 Pumpkin workspace 不再只是参考，而是 Mythicraft 服务端的已授权直接依赖。`server/` 通过 `PumpkinServer` 接入其 Java 网络、Login → Configuration → Play 流程、Anvil 世界、区块、实体、tick 和插件生命周期；`pumpkin-protocol`、`pumpkin-nbt`、`pumpkin-world`、`pumpkin-config`、`pumpkin-codecs`、`pumpkin-plugin-api` 和 `pumpkin-plugin-wit` 作为底座能力复用。

可参考的设计：

- 协议按连接状态和游戏阶段分组。
- NBT 解析、世界信息、Anvil、chunk section、palette、heightmap、光照和 tick scheduler 分层。
- 配置通过 codec 和结构化错误处理，而不是在业务代码中随意读取文本。
- 异步 IO 与 CPU 密集任务分离；CPU 密集任务使用专用线程池，结果通过 channel 回到世界线程。
- Pumpkin 插件 API 继续通过 WIT/component 边界隔离扩展实现；Mythicraft RPG 权威状态不走插件 ABI，而是直接进入 MythicraftCore。

Pumpkin 仓库声明 GPLv3。当前已获得直接使用授权，服务端发布物必须保留 GPL 通知、提供对应源代码及履行其他 GPL 义务；Pumpkin 之外的 Mythicraft crate 仍按各自许可证发布。Steel 仍仅作参考，不因 Pumpkin 授权而改变其许可证边界。

### 2.2 Steel：版本化原版数据和生成纪律

本地 Steel workspace 展示了 `steel-login → steel-core → steel-worldgen → steel-protocol → steel-registry → steel-utils` 的分层，并通过 build 脚本、提取数据和测试资产维护 Minecraft 目标版本。

必须吸收的规则：

- Minecraft 版本由版本矩阵和构建数据共同锁定。
- registry、方块状态、实体类型和协议数据不能靠记忆或手写猜测。
- generated 文件不能直接修改，应修改生成脚本或输入资产。
- 版本行为必须用 fixture、hash 或黑盒测试验证。
- 发生原版行为歧义时，停止猜测，先补充来源数据或 ADR。

Steel 仓库声明 AGPLv3。不能在没有许可证结论的情况下直接拷贝 Steel 源码或生成数据。

### 2.3 MythicMobs：配置是 DSL，不是普通 YAML

本地 MythicMobs 5.13.0 样本覆盖：

- 怪物：Type、Display、Health、Damage、Equipment、Drops、LevelModifiers、Options。
- 技能：触发器、目标选择器、概率、冷却、mechanic、条件和技能组合。
- 物品：原版物品 ID、Display、Lore、Enchantments、Attributes、装备槽和限制选项。
- 掉落：条件、嵌套掉落表、数量范围、概率、经验和 per-player 逻辑。
- 生成：Spawner、随机生成、范围、上限、冷却和玩家聚类。
- 对话：标题、文本、按钮、输入控件、条件和点击后的技能。

因此兼容层必须先解析 AST，再编译为版本化 RPG IR，最后由 Rust 运行时执行。不得把原始 YAML 直接放进 tick 逻辑。

### 2.4 VaultUnlocked：服务语义兼容

VaultUnlocked 的本地代码主要体现 Vault 插件入口、启用监听和经济占位符能力。Mythicraft 应原生提供 Economy、Permission、Chat 和 Placeholder 服务，不加载 `Vault.jar`，也不运行 Bukkit 生命周期。

经济服务必须具备幂等事务 ID、原子提交、审计日志和重放保护。

### 2.5 LuckPerms：权限系统的数据模型

LuckPerms 的本地 API/common 结构提供了成熟权限系统的参考模型：User、Group、Inheritance、Permission、Prefix、Suffix、Meta、Weight、Temporary Node、Context、QueryOptions、缓存、事件和存储适配。

第一版支持这些语义的子集；平台适配和 Java 插件运行不属于目标。

### 2.6 ArcartX 与 DragonCore：客户端内容能力

ArcartX 本地资源和 class 包含 `api`、`command`、`config`、`core`、`event`、`hook`、`network`、`script`、`link`，以及 boss bar、camera、chat card、damage display、entity model、extra slot、font icon、hologram、item effect、key bind、tooltip、waypoint 等资源目录。

DragonCore 本地目录包含插件 jar、客户端 Mod jar、资源和更新说明。它可用于观察 RPG UI、动画、纹理裁剪、模型动画、字体、音频、按键、世界坐标和资源管理能力，但不应直接复用其私有 Bukkit API 或私有 payload。尤其不要把“任意服务端文件同步到客户端”设计为 Mythicraft 基础能力。

当前 ArcartX 兼容落地为 `crates/arcartx` + Pumpkin 原生扫描：核心读取
`plugins/ArcartX/ui` 等目录，将 UI/tooltip YAML/JSON 的原始模型、控件、动作、权限和
资源引用映射到 Mythicraft UI 协议；HUD 页面可自动打开，其他页面通过
`/mythicraft ui <页面ID>` 打开。它不等同于旧 ArcartX `arcartx:main` 加密协议兼容，
也不执行 Aria 脚本、不提供 CRC/签名资源下载；这些能力必须作为独立客户端/资源服务
里程碑验收。

## 3. 版本、许可证和输入资产基线

M0 必须冻结：

| 项目 | 必须冻结 |
| --- | --- |
| Minecraft | Java 版本、协议版本、`DataVersion`、客户端 Mod loader |
| 服务端 | Rust toolchain、目标平台、构建 profile |
| 地图 | Anvil 格式、区块纵向范围、实体/方块实体范围、允许区块区域 |
| RPG | MythicMobs 来源版本、支持的 mechanic/targeter/condition 子集 |
| 权限 | LuckPerms 导入格式、上下文 key、临时节点时间语义 |
| UI | 自有 payload schema、消息上限、资源 manifest 和客户端能力集 |
| 许可证 | 参考代码是否仅分析、是否允许衍生、第三方资产分发边界 |

在许可证审查完成前：

- Pumpkin 代码仅通过已登记的授权路径作为 `mythicraft-server` 运行时依赖使用；不得把 Pumpkin 代码无记录地复制到 Mythicraft 自有 crate。
- Steel 源码、生成数据和资源仍不直接复制，除非另有许可证结论。
- 不分发 MythicMobs、ArcartX、DragonCore、VaultUnlocked、LuckPerms 的 jar。
- 不分发 Mojang 客户端、原版资源或未经授权的音频/模型。
- 只保留来源版本、许可证和测试样本元数据。

## 4. 正式架构

```text
客户端 Mod
  ├─ UI renderer / keybind / asset / audio / model
  └─ Mythicraft Custom Payload
             │
协议与会话层
  ├─ Minecraft protocol / compression / encryption
  ├─ capability negotiation
  └─ validated action ingress
             │
Pumpkin Rust 服务端底座
  ├─ Java protocol / Login / Configuration / Play
  ├─ Anvil/NBT/chunk/entity/tick
  └─ Pumpkin plugin lifecycle / WIT host
             │ MythicraftCore native boundary
Mythicraft RPG 运行时
  ├─ tick-boundary native event hook / Dynamic RPG state
  ├─ RPG IR: skill/condition/effect/damage/loot
  └─ services: economy/permission/chat/placeholder
             │
持久化、日志、指标、管理工具
```

建议 workspace：

```text
crates/
  protocol/ session/ vanilla-data/ nbt/ world/
  simulation/ entity/ rpg/ permission/ economy/
  compat/ client-services/ persistence/ observability/
server/
tools/map-inspector/
tools/config-migrator/
client-mod/
fixtures/
```

### 4.1 世界层

世界边界由 Pumpkin 的世界实现承载，Mythicraft 在其上提供 RPG 领域状态和受控适配：

- `StaticWorld`：由 Pumpkin 读取 `level.dat`、Anvil region、chunk section、palette、heightmap、光照、方块实体和允许区域；RPG 核心不另建第二份世界事实源。
- `DynamicWorld`：玩家、RPG 怪物、掉落物、临时方块、状态效果、任务、经济事件和运行时实体。

第一版不生成新区块。未加载/未允许区块必须拒绝进入、回安全点或断开连接，具体由配置决定。地图读取成功不代表命令方块、数据包、红石、原版 AI 和所有方块实体行为都会工作，支持矩阵必须逐项列出。

方块 palette 数值必须通过目标版本的 registry 还原，不能当作跨版本稳定 ID。

当前 Pumpkin 世界信息实现的已知范围为 `DataVersion 4435..=4903`；地图导入器必须在启动前报告版本、hash 和目录结构，不能把不支持的地图交给运行时后再假定可以降级。对于不支持版本，第一版应输出可读诊断并停止启动。

### 4.2 Tick 和并行模型

第一版沿用 Pumpkin 的单一权威服务端循环和异步外围任务；Mythicraft 领域事件只在约定的 tick 边界提交：

```text
输入收集
 → 规则/技能事件
 → AI/碰撞/效果
 → 生成输出包
 → 原子提交持久化
```

网络 IO、NBT 解压、地图读取、配置编译和资源加载不得同步阻塞权威 tick。路径搜索、批量查询等只读任务可以并行，但结果必须通过 Pumpkin/MythicraftCore 的 tick 提交点提交。

### 4.3 Pumpkin 集成边界

- `server/` 是唯一的真实服务端启动入口，负责 root、Pumpkin 配置、VanillaData、日志、插件初始化和优雅停止。
- Pumpkin 负责 Minecraft 连接、世界加载、玩家/实体生命周期和原版协议；MythicraftCore 直接编译进 Pumpkin 的 Server/ticker/player/payload 路径，不另起一套并行 Login/Play 主循环。
- RPG、Paper 配置、Vault、LuckPerms 和客户端 Mod 能力优先进入 MythicraftCore 的底层服务接口；WIT/插件只保留给非核心扩展，不作为 RPG 权威状态源。
- 任何新增核心字段必须记录线程模型、tick 阶段、持久化边界和 GPL 发行影响，避免在事件层建立第二份玩家或实体事实源。

## 5. 兼容策略

### 5.1 Paper

兼容级别分为文件兼容、语义兼容和二进制/API 兼容。第一版只承诺前两种：

1. `server.properties`、Paper 服务器/世界设置字段导入。
2. 受支持插件配置的 schema、字段映射和诊断。
3. 对无法迁移的 Paper 插件提供 Mythicraft 原生 API 适配指南。

不加载 Paper/Bukkit jar。每个配置族必须有 schema、版本、映射规则、unsupported 列表、dry-run 和 golden fixture。

### 5.2 MythicMobs

- P0：怪物、基础属性、装备、Options、目标选择、基础触发器、伤害/治疗/位移/状态、掉落和经验。
- P1：复杂条件、变量、技能组合、粒子/音效、对话、随机生成、spawner、per-player loot、模型和 UI 事件。
- P2：依赖 Bukkit/NMS/第三方插件的 mechanic、脚本、不能确定等价语义的扩展。

所有 mechanic、targeter、condition 通过 registry 注册，并保留来源文件、YAML 路径、版本和能力要求。

### 5.3 Vault

原生提供 Economy、Permission、Chat、Placeholder；经济事务具备唯一 ID、原子性、审计和重放保护。

### 5.4 LuckPerms

支持 User、Group、Inheritance、Permission Node、负权限、Prefix/Suffix、Meta、Weight、Temporary Node、基础 Context 和查询缓存。导入器需要识别 unsupported backend/字段并生成报告。

## 6. 客户端 Mod 协议

第一版协议消息：

- `hello/capabilities`
- `ui.open/update/close`
- `ui.action`
- `asset.manifest/request/result`
- `audio.play/stop`
- `model.spawn/update/visibility`
- `input.bind/action`
- `combat.damage_display`
- `hologram/bossbar/waypoint`

客户端只渲染和提交用户动作，服务端保持权威。每个 action 带 request ID、页面版本、nonce、能力版本和过期时间。资源只通过 manifest 允许的路径和 hash 发送，不同步任意服务器文件。

## 7. 里程碑与验收

| 阶段 | 交付 | 退出门禁 |
| --- | --- | --- |
| M0 | 版本、许可证、参考审计、样例地图、fixture | 关键决策有 ADR |
| M1 | Pumpkin-backed 协议、登录、会话入口 | 原版客户端可连接，非法包安全拒绝 |
| M2 | Pumpkin 世界加载 + Mythicraft map-inspector/诊断桥 | 真实地图摘要和 chunk 样本可复现 |
| M3 | Pumpkin 静态世界区块服务 + MythicraftCore 原生 RPG 接入 | 出生区、移动、碰撞、重连可用 |
| M4 | Pumpkin tick 接入、实体可见性和 RPG 事件提交 | 固定输入得到确定性状态摘要 |
| M5 | RPG IR 与原生战斗 | 怪物-技能-伤害-掉落闭环 |
| M6 | MythicMobs/Vault/LuckPerms 导入 | 支持项可执行，未知项有报告 |
| M7 | 客户端 Mod UI/资源/音频 | 能力协商、UI action、资源 hash 生效 |
| M8 | 存档、恢复、管理、压测 | 故障注入、重放、安全门禁通过 |
| M9 | 技术预览 | 安装包、样例服、运行手册、兼容表交付 |

## 8. 性能目标

开发基线：20 TPS；固定样例地图和 RPG 压测中 tick p95 ≤ 35 ms、p99 ≤ 50 ms；网络 IO 不阻塞权威 tick；不存在 tick 内同步磁盘 IO；配置和资源重载采用原子切换。

优化顺序：先正确性，再 tracing 定位，再区域分桶/可见性/批量包，最后才考虑 lock-free、SIMD、缓存布局或更激进的并行化。

## 9. 首个垂直切片

```text
真实地图出生区
 → 玩家连接、移动、重连
 → 一个 MythicMobs 风格怪物
 → 一个技能造成伤害
 → 掉落和 Vault 经济奖励
 → LuckPerms 权限判断
 → 客户端 HUD、伤害飘字、音效
 → 恢复玩家状态
```

只有该闭环完成后，才扩展更多原版功能、复杂 mechanic 或高级 UI。
