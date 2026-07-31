# Mythicraft Client Services

Window 3 的 renderer-independent 协议层。当前垂直切片实现：

- schema v1 的公共 payload envelope；
- `hello` / `capabilities` 数据结构和协商；
- namespace、schema、声明长度、消息大小、嵌套深度、数组数量、字符串和 capability 数量限制；
- 未知 schema、伪造长度、缺少必需能力和协议版本不匹配的失败测试；
- `fixtures/client` 下的 JSON golden fixture；
- `ui.open/update/close/action/run` 数据模型；`ui.run` 仅由服务端把已加载配置中的客户端
  UI/Aria 代码转发给 Mod，不接受客户端反向执行请求；
- 按会话使用的 action gate，校验版本、nonce、有效期、权限、距离、状态、重放和固定窗口频率；
- 资源 manifest、稳定 hash、授权来源、安全路径、大小限制和缺失资源 fallback；
- `audio.play/stop` 模型，以及过期、缺失文件和高频事件的客户端降级决策；
- HUD、技能冷却、对话、伤害飘字、bossbar、全息血条、waypoint 和模型可见性模型；
- 组件 revision gate 与缺失 capability/resource 时的明确降级策略；
- 统一 dispatcher、客户端阶段/页面/资源状态机和 transport-neutral smoke runner。

客户端 Minecraft 版本和 Mod loader 尚未在共享版本矩阵冻结，因此本 crate 不绑定 Fabric、NeoForge 或具体 renderer。fixture 中的 `unfrozen` 是显式占位值，不表示已选择 loader。

尚未实现：实际 Minecraft custom payload transport、UI renderer、资源文件传输、真实音频后端、模型 renderer 和游戏内截图 smoke test。
