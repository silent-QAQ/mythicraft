# Mythicraft Server

`mythicraft-server` 是 Mythicraft 的真实服务端入口，当前直接使用已授权的 Pumpkin 核心负责：

- Minecraft Java 登录、Configuration、Play 和协议网络；
- Anvil 世界加载、区块、实体、玩家和 tick；
- Pumpkin 原生配置和插件生命周期。

## 启动

在项目根目录执行：

```text
cargo run -p mythicraft-server -- --root D:\mythicraft\runtime
```

`runtime` 必须是已经存在的目录。首次启动时 Pumpkin 会在该目录生成 `pumpkin.toml` 和默认世界数据；将 `pumpkin.toml` 的 `default_level_name` 指向经过检查的地图目录，即可使用既有地图。入口会在创建 Pumpkin 服务端前读取已有世界的 `level.dat`，提前报告 DataVersion、出生点和损坏/不支持错误。设置 `MYTHICRAFT_MAP_DIAGNOSTIC=1` 后，还会运行一个有界的 Mythicraft 地图诊断，汇总 region、区块数量、DataVersion、坐标范围和损坏 region；默认关闭，避免启动时重复扫描大型地图。当前 Pumpkin 世界数据范围为 `4435..=4903`。

也可以使用环境变量：

```text
$env:MYTHICRAFT_SERVER_ROOT = 'D:\mythicraft\runtime'
cargo run -p mythicraft-server
```

## 当前整合边界

本入口已经把 Pumpkin 的服务器生命周期接入 Mythicraft；RPG、经济、权限和客户端 payload 的第一层核心状态直接位于授权 Pumpkin 源码的 `MythicraftCore`，而配置迁移、持久化和观测仍由 Mythicraft crate 提供。WIT 插件只用于非核心扩展，不持有 RPG 权威状态。

由于该入口直接链接 GPL-3.0 的 Pumpkin 代码，发布 `mythicraft-server` 时必须同时履行 Pumpkin 的许可证义务。详见 `docs/licenses/REFERENCE_SOURCES.md`。

## 构建验证策略

Pumpkin Rust 项目已由维护者实测可构建；本地集成工作只做 Cargo manifest 和格式级检查，不重复执行耗时的 Pumpkin 构建。需要真实构建或集成构建时，提交到 [Mythicraft GitHub 仓库](https://github.com/silent-QAQ/mythicraft)，由 `.github/workflows/ci.yml` 执行。
