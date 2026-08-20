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

### 本地打包安装包

```
cd app
pnpm tauri build --bundles nsis
```

产物落在 `target/release/bundle/nsis/`，一个 `.exe` 安装包加一个 `.sig` 更新签名。
`--bundles nsis` 与流水线一致；不加这个参数会连 MSI 一起打，多拖一次 WiX 下载。

打包要两个环境变量，本机已按用户级配好（新开的终端才读得到）：

| 变量 | 值 |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | `C:\Users\<你>\.tauri\aurora-release.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 空串 |

密码那条即便密钥没设密码也不能省：rsign 始终按「加密私钥」走解密流程，变量缺失时 Tauri 打印
`Decrypting updater signing key, expect a prompt for password` 然后停在交互提示上等输入，
本机实测就这么挂到超时被杀。空串则被当作「未加密私钥」直接放行。

用的就是发布密钥本身，与 `tauri.conf.json` 里的 `pubkey` 成对，所以本地产出的包与流水线产出的
在签名上等价，不会出现那句 `does not match the public key` 的 Warn（Tauri 对不成对只警告不报错，
包照出，装到用户那儿才在校验时失败 —— 流水线把这句 Warn 当错误拦下就是为了防这个）。

### 更新签名密钥

`aurora-release.key` 是**唯一**能给自更新包签名的东西，它只存在于两个地方：本机 `~/.tauri/`，
以及仓库 secret `PRIVATE_KEY`（只写不可读）。两处都丢就再也无法给**已装机的用户**推送更新 ——
只能让所有人手动重装。请另外备份到密码管理器。

2026-08-20 轮换过一次：原密钥本机已无副本，只剩在 secret 里读不出来，等于失去备份。
当时一版都还没发过，换密钥的代价为零；发版之后再换，老用户就永远收不到更新了。
换密钥的动作是「`tauri signer generate` 生成新对 -> `.pub` 文件内容原样填进 `pubkey` ->
私钥内容更新到 secret `PRIVATE_KEY`」，三处必须同时改，改漏任何一处流水线都会在
「构建 NSIS 安装包」那步失败。

## 界面自检脚本

两个脚本管的是编译与单测都看不见的那类问题，改界面前后各跑一次。

```
node scripts/contrast-budget.mjs   # 玻璃材质上的文字对比度实算，改不透明度前必跑
node scripts/page-smoke.mjs        # 逐页逐页签点一遍，抓「整块区域不可见」
```

`page-smoke.mjs` 需要 `npm run dev` 起着，它挂 CDP 驱动一个无头 Chromium 连上去。
之所以要真跑浏览器：framer-motion 的变体编排有没有真的到达子元素，是运行时行为，
tsc / vitest / vite build / clippy / cargo test 五道关全绿的同时，页面可以是整片透明的
（设置页就这么发生过一次，见 `app/src/lib/motion.ts` 的 `tabPanel`）。

## 许可

本项目开源,详见 [LICENSE](LICENSE)。
