# ArcartX 原生 Rust 兼容模块

## 交付范围

crates/arcartx 是根工作区成员，负责读取 ArcartX 常见 UI/tooltip YAML 或 JSON，并产出
原生核心可消费的 Rust 模型。它不直接依赖 mythicraft-client-services，以保持解析器与
协议层解耦；Pumpkin 通过 path dependency 将它内置到服务端核心。

启动时核心会递归扫描以下目录中的 `.yml`、`.yaml`、`.json` 文件：

- `plugins/ArcartX/ui/`（推荐，与原插件目录一致）；
- `plugins/ArcartX/UI/`；
- `plugins/ArcartX/tooltip/` 与 `plugins/ArcartX/assets/{ui,tooltip}/`；
- `config/arcartx/ui/`；
- `config/arcartx/tooltip/`、`config/ArcartX/{ui,tooltip}/`；
- `config/arcartx/`（兼容直接放置页面文件）以及 `arcartx/ui/`、`arcartx/tooltip/`、
  `arcartx/`（兼容直接放置页面文件）。

页面文件名在没有显式 `id` 时就是页面 ID；中文文件名和控件名会原样保留，玩家可用
`/mythicraft ui <页面ID>` 打开。路径必须位于上述目录之一，核心不会递归读取任意服务器目录。

页面完成能力协商后，`ui.isHud: true` 且 `defaultOpen` 未设为 `false` 的页面会自动打开；
其他页面可由玩家执行 `/mythicraft ui <页面ID>` 打开。服务端为每次打开生成新的页面
nonce，并通过 Mythicraft UI 协议信封发送，动作仍由核心负责版本、nonce、权限、过期和
重放保护。

参考源是 D:/mythicraft/ArcartX_Plugin-main；该目录存在，因此没有使用
D:/mythicraft/ArcartX-2.5.36。参考实现确认了以下结构：

- UI 文件名通常承担隐式 id；模块的 source_id 参数会从文件名推导 page_id。
- ui、controls、template、tasks 和 root_control 被映射为页面模型。
- 控件递归保留 type、val、attribute、effect、action、children 和权限。
- 属性/效果中的 ~path 会登记为资源引用；也支持显式 resources/resource。
- 控件动作会生成稳定的 control_id:event 动作 ID，并保留脚本字符串。
- page_id、version、nonce、权限和动作 control_id 均不会静默丢失。
- raw_model 保留 YAML/JSON 解码后的原始键名和整个 JSON 模型；UiOpenDto.model
  直接使用它，不会被 Rust 字段名重写。

## 解析与诊断

\`\`\`rust
use mythicraft_arcartx::{parse_auto, DiagnosticSeverity};

let report = parse_auto(text, Some("ui/character.yml"))?;
for diagnostic in &report.diagnostics {
    if diagnostic.severity == DiagnosticSeverity::Error {
        // 生产接入应拒绝含有 Error 诊断的配置。
    }
    eprintln!("{} {}: {}", diagnostic.code, diagnostic.path, diagnostic.message);
}
\`\`\`

已知字段之外的对象键都会输出 unknown_field 诊断并带路径；开放式的 attribute、
effect、资源 metadata 和动作映射内容属于 ArcartX 扩展空间，不会被当作固定字段误报。
类型不匹配、缺少资源路径、未知动作类型和版本默认值也会输出诊断。解析器不会执行
脚本、表达式、网络下载或客户端动作。

## DTO 接入点

### UiOpen

\`\`\`rust
let open = report.document.to_ui_open_dto()?;
\`\`\`

字段映射：

| ArcartX 兼容模型 | UiOpen 对应字段 |
|---|---|
| document.page_id | page_id |
| document.version | page_version |
| document.raw_model | model |
| document.required_capabilities | required_capabilities |
| document.permissions | required_permissions |

### UiUpdate

\`\`\`rust
let update = report.document.to_ui_update_dto(
    7,
    8,
    [("controls.refresh_button.attribute.texts".to_owned(),
      serde_json::json!("已刷新"))].into_iter().collect(),
)?;
\`\`\`

调用方负责维护当前页面版本；新版本必须大于期望版本，字段集合不能为空。
UiUpdate 的字段路径和 JSON 值不由 ArcartX 脚本自动推导。

### UiAction

\`\`\`rust
use mythicraft_arcartx::ActionEnvelopeContext;

let action = report.document.to_ui_action_dto(
    "refresh_button:click",
    ActionEnvelopeContext {
        request_id: "req-42".to_owned(),
        nonce: Some("fresh-page-session-nonce".to_owned()),
        expires_at_unix_ms: 1_800_000_000_000,
        input: None,
    },
)?;
\`\`\`

UiActionDto 保留 page_id、control_id、动作类型、页面版本、nonce、过期时间、request ID
和可选输入。nonce 优先使用接入上下文注入的值，其次才使用动作/文档中配置的值；静态
配置 nonce 不能替代服务端页面会话绑定、权限检查、重放防护或过期检查。

接入 mythicraft-client-services 时，建议在 client-services 依赖边界内完成：

1. UiOpenDto 的 required_capabilities: Vec<String> 转为 ClientCapability，未知能力拒绝
   或降级并记录原因。
2. UiUpdateDto 直接映射同名字段，并调用目标 crate 的递增版本和 JSON 限制校验。
3. UiActionDto 的动作类型和输入转为目标 crate 的枚举；再用活动页面的 nonce、权限、
   范围、状态和请求去重门控。

## 支持矩阵

| 能力 | 状态 | 说明 |
|---|---|---|
| YAML/JSON 根对象解析 | 支持 | parse_yaml、parse_json、parse_auto。 |
| 页面 ID 与文件名推导 | 支持 | page_id/id 优先；缺失时使用 source_id 文件名。 |
| 页面版本与 nonce | 支持/需接入 | 版本保留；缺失版本诊断并默认为 1；nonce 可保留但生产值应注入。 |
| UI 设置、组件、children | 支持 | 保留常见 ArcartX 字段和开放式属性值。 |
| 原始键名和原始 JSON model | 支持 | raw_model 直接进入 UiOpenDto.model。 |
| template、tasks、tooltip root_control | 部分支持 | 结构和脚本字符串保留，不执行客户端表达式。 |
| 控件动作、显式动作、权限 | 支持 | 支持 click/submit/change/key_press 四类 DTO 动作。 |
| ~resource/path 与显式资源清单 | 支持 | 保留引用、kind、hash、权限和 metadata。 |
| UiOpen/UiUpdate/UiAction/UiRun 字段转换 | 支持 DTO/核心接入 | `UiRun` 只承载服务端已加载配置中的客户端 UI/Aria 代码。 |
| ArcartX 服务端脚本/表达式、packetHandler 执行 | 不支持 | 不执行任意 JVM/JavaScript；UI 动作中的客户端 Aria 代码会通过受保护的 `ui_run` 消息转发给 Mythicraft 客户端 Mod。 |
| 资源下载、CRC/签名链接、客户端渲染 | 不支持 | 当前只提取资源引用并保留在模型中；需要独立资源服务和客户端实现。 |
| 专用 slot/key-bind/model/camera 配置 | 不支持本模块专用解析 | 应由各自兼容模块接入。 |
| 真实 ArcartX 专用网络包互操作 | 未验证 | 当前核心发送 Mythicraft 原生协议；并不声称兼容 `arcartx:main` 的旧加密封包。 |

| Pumpkin 核心启动扫描 | 已接入 | `MythicraftCore::load_from_root` 扫描配置并记录未知字段、类型错误和重复页面。 |
| Mythicraft 客户端 UI 打开 | 已接入 | HUD 自动打开；菜单/非 HUD 页面使用 `/mythicraft ui <页面ID>`。 |
| ArcartX 脚本/表达式执行 | 部分支持 | `console:`/`player:`/`command:` 是显式服务端命令桥；其他 UI 动作正文按客户端 Aria/UI 代码发送，服务端不执行任意脚本。 |

## Fixtures

fixtures/arcartx/ 提供：

- ui-page.yml：页面、嵌套控件、动作、权限、资源和未知字段诊断样例；
- ui-page.json：JSON 页面、components 别名和显式动作样例；
- tooltip.yml：ArcartX tooltip 的 tip/root_control/资源引用样例。

## 未验证项

按任务约束，本地没有执行 cargo build、cargo check 或 cargo test。已执行 `cargo fmt`、
根工作区和 Pumpkin 工作区的 `cargo metadata --no-deps`；真实客户端、完整 ArcartX 版本
覆盖、资源下载、旧 `arcartx:main` 专用协议互操作和最终链接仍须由 GitHub Actions 与
实机客户端验证。
