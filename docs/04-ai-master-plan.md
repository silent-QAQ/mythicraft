# Mythicraft AI 总开发计划

> 用途：四个并行 AI 对话窗口的共同总协议。
>
> 规则：每个窗口只修改自己的责任域；跨域变更先更新契约或提出 ADR，不直接侵入其他窗口目录。

## 1. 项目硬约束

- 第一版固定一个 Minecraft Java 版本、一个协议版本、一个客户端 Mod loader 和一个 `DataVersion` 范围。
- 第一版不做地形生成；只读取经过检查的既有 Anvil 地图和配置允许的区块。
- 服务端保持权威；客户端 Mod 只负责渲染、播放和提交经过校验的用户动作。
- 不运行 Bukkit/Paper Java 插件，不加载 `Vault.jar`、LuckPerms jar 或 ArcartX/DragonCore jar。
- Paper/MythicMobs/Vault/LuckPerms 只做文件和语义兼容子集。
- 外部输入必须限制长度、深度、数量、权限、速率和版本；未知数据不得静默忽略。
- Pumpkin 已获得直接使用授权，作为 `mythicraft-server` 的 GPLv3 运行时底座；必须保留许可证通知、源代码义务和发行记录。Steel AGPLv3 仍只作参考，不直接复制其代码/生成数据。

## 2. 推荐仓库契约

```text
crates/protocol/          Window 1
crates/session/           Window 1
crates/vanilla-data/      Window 1
crates/nbt/               Window 1
crates/world/             Window 1
crates/simulation/        Window 1
crates/entity/            Window 1
crates/rpg/               Window 2
crates/compat/            Window 2
crates/arcartx/           Window 2：ArcartX YAML/JSON 解析与模型映射
crates/permission/        Window 2
crates/economy/           Window 2
crates/client-services/   Window 3
client-mod/               Window 3
crates/persistence/       Window 4
crates/observability/     Window 4
server/                   Window 4
Pumpkin-master/pumpkin/src/mythicraft.rs
                           Window 1：内置 RPG 核心；ArcartX 页面接入遵守 Window 2 契约
Pumpkin-master/pumpkin/src/{server,ticker,net,entity}
                           Window 1：核心生命周期接入
Pumpkin-master/其它源码     已授权底座；按 ADR 修改
tools/                    Window 4
fixtures/                 Shared, changes require review
docs/                     Shared, changes require review
```

如果初始化时选择不同 crate 名称，必须在 ADR 中记录映射，不得让四个窗口各自发明目录。

## 3. 共享类型和契约

窗口之间只通过以下稳定对象交互：

```text
VersionMatrix
WorldSnapshot / ChunkSnapshot
EntityId / PlayerId / ResourceId
TickId / RequestId / TransactionId
RpgDefinition IR
PermissionDecision
EconomyTransaction
ClientCapabilitySet
UiModel / UiAction
AssetManifest
ServerEvent
DiagnosticReport
```

这些对象必须具有：版本字段、明确的所有权/生命周期、序列化 fixture、失败模型和兼容策略。跨 crate 共享类型优先放入 `crates/api` 或独立 schema，不要互相依赖内部实现类型。

## 4. AI 统一执行流程

每个任务按以下顺序：

1. 读取本文件、`docs/03-final-development-plan.md`、自己的分工文档、相关 ADR 和 fixture。
2. 写出范围内、范围外、输入、输出、错误和性能约束。
3. 先增加最小失败测试或 golden fixture。
4. 实现最小垂直切片，不扩展未验收的抽象。
5. 运行最具体测试，再运行受影响 workspace 检查。
6. 更新 schema/ADR/兼容矩阵/运行手册。
7. 报告真实修改文件、真实命令、真实结果和剩余限制。

禁止为了通过测试而删除 fixture、放宽校验、吞掉未知字段或伪造测试结果。

## 5. 统一质量门禁

- `cargo fmt --check`、`cargo check --workspace` 和受影响 crate 测试通过。
- 外部输入无生产路径 `unwrap`/`expect`；`unsafe` 必须有 ADR 和隔离封装。
- 协议、NBT、YAML、权限和经济边界有失败测试。
- 所有性能数字记录机器、版本、地图 hash、配置 hash、玩家/实体数量和 p50/p95/p99。
- 新增协议/IR/配置字段必须更新文档和 fixture。
- 不能证明与原版一致的行为必须标为 `unsupported`、`partial` 或 `divergent`。

## 6. 阶段依赖

```text
Window 1: 版本/协议/世界/实体基础
       ├──────────────┐
       ▼              ▼
Window 2: RPG/兼容   Window 3: 客户端协议/UI
       └──────┬───────┘
              ▼
Window 4: 集成/持久化/压测/发布
```

Window 1 先把 MythicraftCore 编译进 Pumpkin 的玩家、payload、实体和 tick 路径；Window 2 和 3 可在核心契约稳定后并行；Window 4 负责启动入口、CI、fixture、故障测试和基线压测，不要重复实现 Pumpkin 已提供的协议/世界主循环。

ArcartX 接入边界：Window 2 负责 `crates/arcartx` 的配置语义、诊断和动作/资源引用模型，
Window 1 负责把已解析模型挂入 Pumpkin 生命周期，Window 3 负责 Mythicraft 客户端
renderer。旧 ArcartX `arcartx:main` 加密封包、Aria 脚本和 CRC/签名资源服务必须单独
立项并提供实机证据，不能从配置解析结果推导出兼容。

## 7. 里程碑

### M0：共同规格

- 冻结版本矩阵、许可证、地图样本、客户端 Mod loader。
- 记录 Pumpkin、Steel、MythicMobs、LuckPerms、VaultUnlocked、ArcartX、DragonCore 的来源和用途。
- 建立 schema 版本、错误码、ID 命名和 fixture 目录。

### M1：最小可运行核心

- Window 1 完成 Pumpkin 连接、登录、基础区块和 tick 能力，并直接接入 MythicraftCore 的玩家/RPG/payload 契约。
- Window 3 完成 hello/capabilities 的协议样本。
- Window 4 完成 CI、日志、最小黑盒连接测试。

### M2：RPG 垂直切片

- Window 2 完成一个怪物、一个技能、伤害、掉落、权限和经济奖励。
- Window 3 完成 HUD、伤害显示和一个音效事件。
- Window 4 完成重连、存档和垂直切片回归。

### M3：技术预览

- 导入报告、客户端资源 manifest、存档恢复、压测、安全测试和运行手册完整。
- 所有不支持项在兼容矩阵中可见。

## 8. 跨窗口变更协议

如果需要修改其他窗口负责的接口：

1. 在 `docs/adr/` 增加 ADR 或在任务中提出契约变更。
2. 给出旧接口、新接口、迁移方式、影响范围和回滚条件。
3. 先增加跨窗口 fixture，再修改实现。
4. 不直接重写他人责任域的代码来“顺手修复”。

## 9. AI 任务模板

```text
任务 ID：MC-XXXX
窗口：1/2/3/4
目标：一个可观察结果
依赖：版本、crate、fixture、前置任务
范围内：明确文件/模块
范围外：明确不实现内容
契约：输入、输出、错误、版本、权限
验收：Given/When/Then
测试：unit/integration/golden/fuzz/bench/manual
交付：代码、fixture、文档、实际验证结果
```

## 10. 启动提示

```text
你正在参与 Mythicraft 的并行开发。先阅读 docs/03-final-development-plan.md、
docs/04-ai-master-plan.md 和你所属窗口的文档。你不是独自在仓库中工作，
不要撤销其他窗口的改动，只修改责任域文件。第一版固定版本、不做地形生成、
不运行 Java Paper 插件。所有外部输入必须有 schema、长度、深度、权限和版本校验。
先写 fixture/test，再实现代码；完成后报告真实的文件、命令、结果和限制。
```
