# AI 窗口 2：RPG、Paper 配置与兼容服务

## 责任域

负责 MythicMobs 风格 RPG 运行时、Paper 配置导入、Vault 服务语义、LuckPerms 权限语义、配置诊断和版本化 RPG IR。

负责目录：`crates/rpg`、`crates/compat`、`crates/permission`、`crates/economy`、`fixtures/compat`。

不负责：Minecraft 底层协议/区块、客户端 Mod renderer、全局 CI 和发布压测基础设施。

## 语义边界

兼容分为：文件兼容、语义兼容、二进制/API 兼容。第一版只承诺前两种，不运行 Paper、MythicMobs、Vault 或 LuckPerms jar。

## 实施顺序

### R1：统一诊断和 AST

- YAML 读取保留文件、行列和字段路径。
- 类型错误、未知字段、未知 mechanic、深度/数量超限都有结构化 code。
- 导入支持 `dry-run`，输出 `supported/converted/partial/unsupported/invalid`。

验收：任意未支持字段都能定位到源文件和路径；不能静默丢字段。

### R2：RPG IR

定义并版本化：

- `RpgEntityDefinition`
- `AttributeDefinition`
- `SkillDefinition`
- `Trigger`
- `TargetSelector`
- `Condition`
- `Effect`
- `DamageEvent`
- `ItemDefinition`
- `LootTable`
- `DialogDefinition`

运行时只消费 IR。IR 必须可序列化、可校验、可 hash、可缓存和可诊断。

### R3：MythicMobs P0

实现本地 5.13.0 样本中最小闭环：怪物 Type/Display/Health/Damage、装备、Options、基础目标、触发器、技能组合、伤害/治疗/位移/状态、掉落、经验。

将 mechanic、targeter、condition 设计为 registry；每个实现声明版本、输入 schema、权限要求、tick 成本和不支持条件。

验收：一个怪物能生成、索敌、施法、造成伤害、死亡、掉落和经验；同一随机种子/fixture 可复现结果。

### R4：Vault 与经济

原生 Economy 服务至少支持余额查询、存款、取款、转账、货币名称、错误结果和审计查询。

每个交易包含 `TransactionId`、玩家、原因、金额、前后余额、tick、配置 hash 和幂等状态。重试不能重复发钱；失败不能半提交。

兼容 VaultUnlocked 的服务概念和常见占位符，不加载 Java plugin。

### R5：LuckPerms 语义子集

支持 User、Group、Inheritance、Permission Node、负权限、Prefix/Suffix、Meta、Weight、Temporary Node、基础 Context 和缓存查询。

权限计算必须定义继承顺序、负权限优先级、权重选择、临时节点过期和上下文缺省行为。导入器对不支持的 backend/字段给报告。

### R6：Paper 与配置迁移

- 导入 `server.properties` 和选定 Paper 服务器/世界字段。
- 为 MythicMobs、Vault、LuckPerms 分别建立 schema 和版本适配器。
- 每个适配器有 raw → AST → IR → diagnostics golden fixture。
- 迁移失败时保留上一份有效配置，禁止半重载。

## P0/P1/P2 支持矩阵

- P0：基础怪物、属性、伤害、技能触发、目标、物品、掉落、经济和权限。
- P1：复杂条件、变量、对话、spawner、随机生成、per-player loot、粒子/音频/UI 事件。
- P2：Bukkit/NMS/第三方插件脚本、任意 Java mechanic、无法证明等价行为的扩展。

## 跨窗口契约

从 Window 1 接收：`EntityId`、目标查询、`TickContext`、服务端伤害/效果入口、实体快照。

向 Window 3 提供：`ServerEvent`、UI model、combat display event、audio event、dialog action schema。

向 Window 4 提供：导入 CLI、golden report、战斗回放 fixture、经济审计查询和权限测试场景。

## 安全和性能

- YAML 锚点、递归、深度、列表数量、字符串长度和表达式复杂度有限制。
- 技能执行不得执行任意代码、任意文件 IO 或任意网络请求。
- 每 tick 的技能预算、递归 skill 深度和事件数量必须有上限。
- 外部请求不能直接写生命、掉落、权限或余额。

## 交付格式

报告支持的原始配置版本、字段覆盖率、unsupported 清单、IR 变更、fixture、测试命令和已知语义差异。不得使用“兼容 MythicMobs”作为没有支持矩阵的结论。

