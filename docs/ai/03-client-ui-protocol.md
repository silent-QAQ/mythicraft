# AI 窗口 3：客户端 Mod、UI 与资源协议

## 责任域

负责客户端 Mod、自定义 Payload、UI 模型、资源 manifest、贴图/字体/模型引用、音效播放、按键、伤害显示、全息、bossbar、waypoint 和客户端能力协商。

负责目录：`client-mod`、`crates/client-services`、`fixtures/client`、客户端资源 schema。

ArcartX 配置映射由 `crates/arcartx` 与 Pumpkin 核心共同承载：窗口 3 需要维护映射后的
客户端 renderer/协议契约，不能把它误当作旧 ArcartX 专用 `arcartx:main` 网络协议；
窗口 2/4 负责解析诊断、服务端接入和实机互操作证据。

不负责：服务端权威战斗计算、经济变更、权限计算、Anvil 解码和 Rust tick 核心。

## 参考边界

ArcartX 的 class/资源目录和 DragonCore 的插件/Mod/更新说明可用于提炼功能清单和用户体验，不直接复用其私有协议、Bukkit API、license 校验、jar 或任意服务端文件同步机制。

第一版只选一个 Minecraft 版本和一个 Mod loader。协议模型独立于具体 renderer，避免服务端被客户端 GUI 实现绑定。

## 实施顺序

### C1：能力协商

客户端 hello 至少包含：协议版本、Mod 版本、loader、资源 manifest hash、支持的 UI/audio/model/input 能力。

服务端响应 accepted capabilities、required capabilities、降级能力和错误原因。不满足必需能力时拒绝进入 RPG play state。

### C2：Payload 公共封装

每条消息包含：namespace、message type、schema version、request ID、payload length、可选 nonce/expiry。

服务端和客户端都限制消息长度、嵌套深度、数组数量和频率。未知版本不能被当作旧版本静默解析。

### C3：数据驱动 UI

实现：

- `ui.open`：页面 ID、页面版本、模型、权限/能力要求。
- `ui.update`：局部字段更新和版本号。
- `ui.close`：原因和清理策略。
- `ui.action`：控件 ID、动作类型、页面版本、nonce、输入值和 request ID。
- `ui.run`：服务端在完成 `ui.action` 权限与重放校验后，向客户端发送配置中声明的 Aria/UI
  代码；该消息只允许服务端发出，客户端不得反向请求执行服务端命令。

服务端不执行任意脚本，也不接受客户端上传代码；仅把已加载配置中的 UI 动作正文作为
`ui.run` 转发给客户端，客户端必须对该消息做来源、页面 nonce 和脚本能力限制。action
必须在服务端校验权限、页面版本、nonce、距离/状态和速率。

ArcartX 页面接入约定：`plugins/ArcartX/ui/**/*.yml` 或 `.json` 的 `ui.isHud: true` 页面
在能力协商后自动打开；非 HUD 页面由 `/mythicraft ui <页面ID>` 触发。`raw_model` 保留
ArcartX 的 `root_control`、`attribute`、`effect`、`action` 等原始键名，未知字段只做
诊断，不执行脚本或表达式。

### C4：资源与音频

资源 manifest 包含资源 ID、类型、路径、hash、大小、版本和授权来源。客户端只加载允许的资源，缺失资源有明确降级。

音频事件包含 sound ID、位置/跟随实体、音量、声道、优先级、过期时间和客户端限流。不得因不存在的音频文件导致客户端卡死或断线。

### C5：RPG 体验组件

第一批组件：HUD、伤害飘字、技能栏/冷却、对话、bossbar、全息血条、waypoint、一个模型可见性事件和一个音效事件。

后续再增加 camera、extra slot、复杂动画、脚本化 UI 和高频粒子批处理。

## 协议事件

```text
hello/capabilities
ui.open/update/close/action/run
asset.manifest/request/result
audio.play/stop
model.spawn/update/visibility
input.bind/action
combat.damage_display
hologram/bossbar/waypoint
```

## 跨窗口契约

从 Window 1 接收：协议包通道、玩家/实体快照、能力协商入口、服务端事件和 tick 时间。

从 Window 2 接收：UI/对话/战斗/经济/权限事件；Window 3 不决定事件是否真实发生。

向 Window 4 提供：客户端 smoke test、协议 golden fixture、伪造 action、重放、过期 action、资源 hash 不匹配和高频音频测试。

## 安全要求

- 客户端不能直接提交伤害、掉落、余额、权限或传送结果。
- action 必须有 nonce、版本和有效期；重复请求必须幂等或拒绝。
- 所有文本、路径、资源 ID 和模型参数有长度/字符集限制。
- 服务端不执行客户端下发脚本，不允许任意 URL 资源。
- UI 权限由服务端决定，客户端隐藏控件不等于权限校验。

## 交付格式

报告客户端版本、loader、schema 版本、能力矩阵、资源来源、协议 fixture、手工截图/录屏结果和降级行为。不得声称“支持 ArcartX/DragonCore”而没有明确自有协议映射表。
