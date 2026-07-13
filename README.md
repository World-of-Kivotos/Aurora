# Aurora

自研 Minecraft: Java 版启动器,Rust 后端 + iced 原生 GUI,面向中国玩家,注重低配机友好与国内网络下的下载体验。

> About (English): Aurora is a self-developed, open-source launcher for Minecraft:
> Java Edition, written in Rust with a native iced GUI. It handles genuine Microsoft
> account login via the standard OAuth2 device code flow, downloads and installs the
> game (with BMCLAPI mirror support for users in China), manages Java runtimes, mod
> loaders (Fabric/Quilt/Forge/NeoForge) and mods (Modrinth/CurseForge), and launches
> the game. It does not bypass, weaken, or disable any authentication, licensing, or
> security checks, and complies with the Minecraft EULA.

## 项目状态

早期开发中(WIP)。当前处于后端搭建阶段:Cargo workspace 与分层 crate 结构已就绪,核心模块正在实现;iced 前端待后端稳定后启动。

## 技术栈

- 后端:Rust(Cargo workspace,tokio 异步运行时,reqwest + rustls 网络,thiserror 错误处理)
- 前端(规划中):iced 0.14 原生 GUI(非 WebView,无浏览器依赖,自带软件渲染回退以兼容老核显)
- 目标平台:Windows 10 1803+(Rust msvc 目标下限)

## 架构

后端采用分层 workspace,依赖自上而下单向流动:

```
aurora-base                              公共设施(HTTP/镜像/校验/路径)
  -> aurora-auth / aurora-version / aurora-java / aurora-download
    -> aurora-instance / aurora-install / aurora-modplatform
      -> aurora-launch                   启动链路
        -> aurora-core                   门面(对外统一 API)
          -> aurora-cli                  调试用命令行
```

详见 [docs/architecture.md](docs/architecture.md)。功能范围来自对 PCL2 的全量盘点,见 [docs/pcl2-feature-inventory.md](docs/pcl2-feature-inventory.md)。

## 核心功能(规划)

- 账户:微软正版登录(设备码流)、离线账户、Authlib-Injector 第三方登录,DPAPI 加密存储令牌
- 版本:版本清单解析、inheritsFrom 继承合并、加载器识别、版本隔离
- 下载:多线程分块下载引擎,官方源与 BMCLAPI 镜像自动切换,断点续传与校验
- Java:自动探测、按版本匹配、自动下载运行时
- 安装:原版本体与资源补全,Fabric/Quilt/Forge/NeoForge 自动安装
- Mod:Modrinth 与 CurseForge 双平台聚合搜索,本地 Mod 管理
- 启动:参数拼装、进程管理、崩溃基础诊断

## 微软账户登录

Aurora 使用标准的 Microsoft OAuth2 设备码流(公共客户端,不含密钥,scope `XboxLive.signin offline_access`)让玩家用自己的正版账号登录,仅读取 profile 与所有权信息用于下载启动玩家正版拥有的游戏,不绕过任何验证、授权或安全检查。

## 构建

```
cargo build --workspace
cargo test --workspace
```

## 许可

本项目开源,详见 [LICENSE](LICENSE)。
