# AI 窗口 4：集成、持久化、测试与发布

## 责任域

负责 workspace 初始化与 CI、`server/` 入口装配及 Pumpkin 内置 MythicraftCore 的集成验证、fixture 管理、黑盒集成、持久化、崩溃恢复、性能压测、模糊测试、安全门禁、运行手册和技术预览发布。

负责目录：`crates/persistence`、`crates/observability`、`tools`、`tests`、`.github`、`fixtures` 的测试部分、发布文档。

不负责：重写 Window 1/2/3 的业务实现；发现问题时优先添加回归 fixture 和最小修复请求。

## 实施顺序

### I1：Pumpkin 底座装配、工程骨架和 CI

- 维护外层 Mythicraft workspace 与内层 Pumpkin workspace 的边界；确保 `server/` 通过授权路径依赖 Pumpkin，不把 Pumpkin workspace 依赖错误继承到 Mythicraft 根。
- 建立 workspace、fmt、check、clippy、test、依赖审计和文档检查；Pumpkin 本体构建由已授权底座的独立验证流程负责，集成验证不得伪造其结果。
- 固定 Rust toolchain、构建 profile、目标平台和可复现构建元数据。
- 记录参考项目许可证和输入样本来源。

初始门禁：格式、编译、单元测试、许可证清单和禁止分发文件扫描。

### I2：测试 fixture 系统

目录建议：

```text
fixtures/
  version/
  protocol/
  world/valid world/corrupt world/unsupported
  compat/mythicmobs compat/vault compat/luckperms
  client/
  replay/
```

每个 fixture 记录来源、版本、hash、预期结果和许可状态。Golden 变更必须有人类可读的变更说明。

### I3：持久化与恢复

实现玩家位置、属性、背包/物品、任务、权限缓存和经济状态的版本化存档。

要求：原子写入、临时文件/日志、版本迁移、备份、恢复命令、交易审计和重复提交保护。配置重载失败时保留上一份有效配置。

故障注入：写入中断、进程强杀、磁盘空间不足、重复事务、半写文件、旧版本 schema 和玩家同时重连。

### I4：黑盒集成

测试闭环：

```text
启动 Pumpkin-backed server → 读取地图 → 客户端连接 → 能力协商 → 玩家移动
 → RPG 怪物 → 技能/伤害 → 掉落/经济 → UI/音效
 → 断线重连 → 存档恢复
```

每个阶段都要有机器可读结果和失败日志，不只依赖截图。

### I5：安全与模糊

- 协议：截断包、非法 VarInt、超长消息、未知版本、重放和 flood。
- NBT/Anvil：损坏压缩、递归、超大列表、异常 palette、未知 DataVersion。
- YAML/配置：深度、别名、超长字符串、未知 mechanic、表达式复杂度和路径逃逸。
- Client action：伪造、过期、重复、越权、跨页面和频率超限。
- 经济：重复扣款、重复奖励、断线重试、负数、溢出和审计不一致。

### I6：性能基线

固定记录：机器、OS、Rust profile、目标版本、地图 hash、配置 hash、玩家数、实体数、技能频率、测试时间、p50/p95/p99、内存峰值和网络吞吐。

开发门禁：20 TPS；tick p95 ≤ 35 ms、p99 ≤ 50 ms；网络 IO 不阻塞 tick；地图和配置加载不在 tick 内同步执行。

压测场景至少包括：静态玩家移动、实体密集、技能高频、多人可见性广播、区块批量发送、经济高并发、UI/音频高频事件。

### I7：发布和运行手册

发布包必须包含：服务端二进制、配置样例、地图检查器、配置迁移器、客户端 Mod 版本、资源 manifest、兼容矩阵、备份/恢复命令、故障排查和已知限制。

不得打包第三方插件 jar、Mojang 资产或没有许可记录的资源。

## 跨窗口验收

从 Window 1 获取协议/地图/状态摘要测试；从 Window 2 获取导入报告、战斗回放、经济审计和权限场景；从 Window 3 获取客户端 smoke、payload golden 和资源 hash 测试。

发现问题时：

1. 先生成最小可复现输入和回归 fixture。
2. 判断责任窗口和是否违反共享契约。
3. 提交具体修复任务，不直接覆盖其他窗口的实现。
4. 修复后执行原测试与相关全链路测试。

## 发布前清单

- [ ] 版本矩阵、许可证和输入资产来源已归档。
- [ ] 地图检查、配置迁移和客户端能力协商可重复执行。
- [ ] 未支持的 Paper/MythicMobs/LuckPerms/Vault 语义已列出。
- [ ] 玩家、经济、物品、任务存档有恢复测试。
- [ ] 协议、NBT、配置、action 和经济有模糊/重放测试。
- [ ] 性能报告包含完整环境元数据。
- [ ] 安装包不包含未授权第三方 jar/资产。
- [ ] 运行手册能在干净环境启动样例服。

## 交付格式

报告 CI 检查、测试命令和实际输出摘要、压测环境和数据、失败样本、已知风险、发布包内容和下一阶段建议。没有实际运行的命令不得写成“通过”。
