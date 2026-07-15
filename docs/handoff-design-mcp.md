# Aurora 交接文档（2026-07-15）：状态 + 设计类 MCP 选型笔记

本文件用于开新对话时快速接续。分两部分：一是当前工程状态，二是"给设计链路接入 MCP 搞 UI 创新"这个待办的选型笔记。

选型笔记原为凭知识的候选清单（满篇 [需联网核实]），已于 2026-07-15 用动态工作流（12 个研究员并行 + A/B 档对抗式复核，共 20 个子代理）联网核实完毕：每一项的安装命令、当前版本、维护活跃度均以 GitHub REST API 原始 JSON、raw README 原文、npm registry time 字段等一手数据源逐字核对，不采信 WebFetch 摘要（复核中实测发现摘要模型多次把 2026 年份幻觉成 2024，见文末附录）。下文所有命令与版本可直接落地，标注了每项的复核结论（确认 / 需修正）。

---

## 一、当前工程状态

### 后端（已完成，未动）
- aurora-core 共 11 个 crate 全部实现，`cargo test --workspace` 395 passed，clippy 零告警，已推送到 github.com/World-of-Kivotos/Aurora。
- 门面 `Aurora` 关键公开方法：
  - `Aurora::load()`（async 构造）、`config()`、`game_dir()`、`data_dir()`
  - `set_game_dir(&mut)`、`set_client_id(&mut)`、`save_config(&self, async)`
  - `list_manifest()`、`list_installed()`（async）
  - `create_offline_account(name, events)`（跨平台）
  - 仅 Windows：`microsoft_login`、`accounts`、`current_account`、`set_current_account`、`remove_account`
  - 事件模型：`EventSink = mpsc::UnboundedSender<CoreEvent>`；`CoreEvent = Stage|Warning|Download`

### 前端（已从 iced 切到 Tauri，地基提交 bf5cc05）
- `app/` = Vite + React + TS + framer-motion；`app/src-tauri/` = `aurora-tauri` crate（path 依赖 aurora-core，已纳入 workspace）。
- 门面进 `tokio::sync::Mutex<Aurora>` managed state —— 兼容 `&self` 与 `&mut self`，补上 iced 时代够不到的持久化入口。
- 四条 IPC 命令：`get_config` / `list_installed` / `current_account` / `create_offline_account`。
- DTO 剥离账户令牌，绝不整体过 IPC；进度事件走 `mpsc -> Tauri emit` 统一范式，事件名 `aurora://core-event`，供后续 install/launch 照抄。
- 设计系统：`app/src/styles/theme.css`（纯白 + 蓝粉渐变 token，单一事实来源）、`app/src/lib/motion.ts`（弹簧预设 tap/settle/pop/soft/morph/aurora）、麦麦式固定图标侧栏（`layoutId` 浮起胶囊）、Card/Button/PageHeader/EmptyState 组件、HashRouter 外壳。
- 主页端到端接 `list_installed` + `current_account`。
- 验证：`pnpm build`（tsc + vite 零错）与 `cargo build -p aurora-tauri` 双绿（本人亲验，非仅 agent 报告）。
- 视觉法则见 `docs/ui-visual-spec.md`；地基详细约定见地基 agent 交付报告（在上一对话里）。

### 本地怎么跑
```
cd D:\Repo\Aurora\app
pnpm install
pnpm tauri dev
```

### 待办（任务列表）
- #5 后端集成验证 M2（真机端到端：装 1.21.x+Fabric、离线启动、设备码登录）
- #7 前端页面接真实 IPC（账户/设置/主页启动）
- #8 扩 aurora-core 门面缺失写入口（离线账户持久化 upsert、正版启动、Forge/NeoForge、install/launch 进度贯通）

### 一个已确认的环境坑（截图）
应用内浏览器面板（mcp__Claude_Browser__）指到 `localhost:1420` 后，`read_page` 能读到实时 DOM，但 `screenshot` 一律 30s 超时。JS 探针查明：该标签页 `document.visibilityState = "hidden"`，浏览器对隐藏标签节流 paint/rAF，所以截不到帧 —— 这是面板未被前置的环境问题，**不是** app 的动画 bug（framer-motion 正常）。要真正"看见 UI"，用能驱动独立真实浏览器的方案（见下方 Playwright MCP）。

---

## 二、设计/审美类 MCP 选型笔记（已联网核实，2026-07-15）

说明：本机内置 MCP 注册表（mcp__mcp-registry__）实测对 figma/design/slack/github 等全部返回空、`list_connectors` 显示未装任何连接器，所以枚举靠不了它，本轮改用联网调研 + 一手数据源逐字核对。核实覆盖 12 个候选（含 2 个原文档未列、经资深视角补入的高相关项：shadcn 官方 MCP、Chrome DevTools MCP）。

### 核实结果速查表

| 候选 | 官方 | 复核 | 最新版本 / 最近活动 | 档位 |
|------|------|------|--------------------|------|
| Playwright MCP | 微软 | 确认 | 0.0.78（2026-07-09；main 今日仍有提交） | A |
| Figma 官方 Dev Mode MCP | Figma | 确认 | 托管服务无版本号；指南仓库 2026-07-13 | A |
| Framelink Figma MCP（GLips） | 社区一手 | 需修正命令 | 0.13.2（2026-06-18；main 2026-06-24） | A |
| 21st.dev Magic MCP | 21st.dev | 需修正命令 | npm @latest=0.1.0（2025-06；GitHub 2026-02-17） | B |
| shadcn 官方 MCP | shadcn | 确认 | 4.13.0（2026-07-03，约每周发版） | B |
| Chrome DevTools MCP | Google | 确认 | 1.6.0（2026-07-14，46977 star） | B |
| Replicate 官方文生图 MCP | Replicate | 确认 | 托管服务；平台 2026-04 仍更新 | B |
| Unsplash MCP（hellokaton） | 个人 | 确认（无 CC 一键接入） | 0.1.0（2026-04 仅合并徽章 PR） | B |
| Iconify MCP（imjac0b） | 个人 | 确认 | 1.0.4（2025-11 后停更 8 个月） | C |
| 灵感画廊聚合类 | 个人 | 确认（无可靠实现） | 唯一实现 3 提交/同一天/9 star | C |
| 配色/设计令牌类 | 社区零散 | 确认（生态不成熟） | 均个位数 star 个人项目 | C |
| Blender MCP（已连） | ahujasid | 确认（活跃但另立线） | 1.6.4（2026-06-11；main 2026-07-14） | 3D 素材线（可选） |

"需修正命令"指研究阶段给出的命令被对抗式复核逐字驳回并订正，正确命令见下方各条 —— 落地时以下方为准。

### 结论先行：建议今晚起手的最小集
1. Playwright MCP（微软官方，档A，确认）—— 直接解决"AI 看不见自己做的 UI"这个当前最痛的点，驱动独立真实浏览器截图/读 DOM/点击，指到 `localhost:1420` 就能让 AI 真看到界面并据此迭代。开源免费、零鉴权、今晚即可接。首选。
   ```
   claude mcp add playwright npx @playwright/mcp@latest
   ```
2. Chrome DevTools MCP（Google 官方，档B，确认，可选补充）—— 若要 DevTools 级性能 trace / 内存快照 / 网络细节深调，作 Playwright 的互补件（两者定位不同、不是二选一）。注意官方仅保证真 Chrome 与 Chrome for Testing，不保证覆盖 Tauri 桌面壳的 WebView2（Windows 为 Edge Chromium 内核），所以它稳的是"用真 Chrome 打开 Vite 页面调试"这一子场景，不能直接截启动器窗口。
   ```
   claude mcp add chrome-devtools --scope user npx chrome-devtools-mcp@latest
   ```
3. Blender MCP（本会话已连，档 3D 素材线）—— 做 3D hero/皮肤预览渲染，给"华丽游戏化"上真 3D，用法见"创新方向"。本身维护活跃（v1.6.4），只是它服务的是 3D 资产产线而非 React/CSS 样式闭环，故与上面两项分线看待。

延后（当"结构参考弹药"而非即用成品）：21st Magic 与 shadcn 官方 MCP 都能生成现代 React 组件，但产物默认是 Tailwind + class-variance-authority 写法，而 Aurora 前端（已核 `app/package.json`）是纯 React + framer-motion + CSS Modules、未引入 Tailwind/shadcn 任何依赖 —— 直接装入的组件必须整体重写样式层才能落到我们的 `theme.css` token 体系。接这两个前，先决定 Aurora 是否引入 Tailwind/shadcn 风格；不引入的话，它们的价值只在"结构/交互/可访问性模式参考"。

### 分档候选（逐条核实结论）

档 A：推荐接入
- Playwright MCP（`@playwright/mcp`，微软官方）。复核确认：README 的 Claude Code 小节原文即 `claude mcp add playwright npx @playwright/mcp@latest`（注意此条不带 `--` 分隔符，是官方对 Claude Code 的专属写法）。Apache-2.0 开源免费、无需 API key。给 Aurora：截图/观察运行中的前端做闭环视觉迭代，也能抓参考站截图供"取其形"。风险：首次运行会自动下载浏览器内核（上百 MB 量级），受限网络需提前规划；npm 上残留大量 `1.52.0-alpha-*` 旧版本号，务必用 `@latest` 或锁 0.0.78 别拉到旧 alpha；默认开可见浏览器窗口（非无头）。
- Figma 官方 Dev Mode MCP Server。复核确认（原文档判档B，核实后上调档A：官方活跃、接入路径明确）。仍是公开 Beta（未 GA），提供两条接入：
  ```
  # 推荐：Anthropic 官方插件市场，一并装 MCP 配置 + Agent Skills
  claude plugin install figma@claude-plugins-official
  # 远程服务器（走 Figma OAuth，无需装 Figma 桌面 App）
  claude mcp add --transport http figma https://mcp.figma.com/mcp
  # 桌面服务器（读 Figma 桌面 App 当前打开的文件；需先在 Dev Mode 面板点 Enable desktop MCP server）
  claude mcp add --transport http figma-desktop http://127.0.0.1:3845/mcp
  ```
  Beta 期免费，write-to-canvas 等写入能力官方声明未来转按量收费，桌面写入需付费方案下的 Dev/Full 席位。**有条件**：价值前提是团队确实在 Figma 里维护设计稿；当前仓库最近提交都是直接改 React/Tauri 代码调配色，没有 Figma 设计源的迹象，若不走 Figma 则此项空转。注意 GitHub 上 `figma/mcp-server-guide` 是官方"接入指南"仓库（非 server 源码，server 是闭源托管服务）。
- Framelink Figma MCP（GLips/Figma-Context-MCP，npm 包 `figma-developer-mcp`）。复核需修正命令：研究阶段误用 npm 包名作服务器标识符，官方 quickstart 的 Claude Code 标签页原文用的是别名 `Framelink_Figma_MCP`：
  ```
  claude mcp add Framelink_Figma_MCP -- npx -y figma-developer-mcp --figma-api-key=YOUR-KEY --stdio
  ```
  v0.13.2（2026-06-18 发版，main 分支 2026-06-24 仍有提交），MIT 免费，仅需一枚 Figma 个人访问令牌（PAT，Figma 账号免费自助生成，不要求付费套餐）。与官方 Figma MCP 是两套独立产品，二选一即可：官方版走 OAuth/托管、能力更全但 Beta 转收费；Framelink 走 PAT/本地、纯 REST 拉数据、免费。--figma-api-key 是明文凭据，建议改用 `FIGMA_API_KEY` 环境变量避免写进会入库的配置。

档 B：可试用/有条件
- 21st.dev Magic MCP（`@21st-dev/magic`）。复核需修正命令 + 重大隐患。命令修正：Claude Code v2.1.0 起参数解析回归，选项（含 `--env`）必须排在服务器名之前，且 `--env` 与服务器名之间要隔一个其它选项，否则 CLI 会把服务器名当成又一个 KEY=value 报错。正确形态：
  ```
  claude mcp add --env API_KEY="<你的21st.dev API Key>" --transport stdio magic -- npx -y @21st-dev/magic@latest
  ```
  隐患一：npm `@latest` 仍指向 2025-06-09 的 0.1.0（超 13 个月未更新），2026-02-17 的安全补丁（升 @modelcontextprotocol/sdk 到 ^1.25.3）只在 GitHub、从未发 npm，照此命令实际装到的是旧构建；接入前需自行盯梢版本。隐患二：纯云端代理，会把你输入的 UI 描述发到 21st.dev 服务器，需评估隐私。隐患三：产物强绑 Tailwind + shadcn 工具链，与 Aurora 现有 CSS Modules 体系错位（见"最小集"末段）。API key 在 https://21st.dev/magic/console 生成。
- shadcn 官方 MCP（npm 包 `shadcn` 内置 `mcp` 子命令，来自 shadcn-ui/ui 单体仓库）。复核确认。官方规范路径是生成项目级 `.mcp.json` 而非全局 add：
  ```
  npx shadcn@latest mcp init --client claude
  ```
  已知未修官方 bug（issue #9181，2025-12 提出、至今 Open）：生成的 `.mcp.json` 缺 Claude Code 规范要求的 `"type":"stdio"` 字段会连不上，需手动补，手动配置应为：
  ```
  {"mcpServers":{"shadcn":{"type":"stdio","command":"npx","args":["shadcn@latest","mcp"]}}}
  ```
  v4.13.0（2026-07-03，近两月约每周发版，非常活跃），官方公开注册表免费无鉴权。价值同 21st Magic：结构/可访问性参考为主，样式必须整体重写成我们的 token 体系。
- Chrome DevTools MCP（Google 官方，命令见"最小集"第 2 项）。复核确认。v1.6.0（2026-07-14），Apache-2.0，Puppeteer 驱动真实 Chrome，长于 DevTools 级深度观测（性能 trace/内存 heapsnapshot/网络细节）。默认向 Google 上报使用统计、性能 trace 可能发往 CrUX API，调试未发布界面时加 `--no-usage-statistics --no-performance-crux` 关闭。不保证覆盖 Tauri 的 WebView2（见"最小集"）。
- AI 文生图类 —— Replicate 官方 MCP（远程托管）。复核确认，同时确认原文档提到的 EverArt MCP 已废弃（官方仓库 2025-05-29 归档只读，勿再采用）。
  ```
  claude mcp add replicate https://mcp.replicate.com/sse --transport sse --scope user
  # 之后启动 claude，执行 /mcp 完成 Replicate API Token 认证
  ```
  平行备选 fal.ai 官方 MCP（模型库更大、免 OAuth，用 Bearer Token，更适合脚本化）：
  ```
  claude mcp add --transport http fal-ai https://mcp.fal.ai/mcp --header "Authorization: Bearer YOUR_FAL_KEY"
  ```
  两者 MCP 本体免费、按量计费无免费额度（Replicate 的 FLUX 1.1 Pro 约 $0.04/张、FLUX Dev $0.025/张）。给 Aurora：生成极光背景/概念插画适配度高、开箱可用；但要产出 UI 里可直接平铺的无缝材质贴图，通用文生图不保证 seamless tiling，需额外 prompt 工程或后处理，别指望一次出生产级素材。海外托管，未验证国内直连稳定性。
- Unsplash / 图库类 MCP。复核确认：目标仓库 hellokaton/unsplash-mcp-server 真实且近期有动静，但**无官方 Claude Code 一键接入**（README 只覆盖 Cursor/Windsurf/Cline 与经 Smithery 装 Claude Desktop，最近提交全是合并徽章 PR、功能性 issue 已停滞响应）。需 `UNSPLASH_ACCESS_KEY`（unsplash.com/developers 免费申请），但仓库文档未交代图片商用授权边界，需自行查 Unsplash License（unsplash.com/license）。若确要接，同名生态里 `@jeffkit/unsplash-mcp-server`（npm，`npx -y @jeffkit/unsplash-mcp-server --access-key KEY`）更贴近 npx 一键形态，但本轮未做同等深度核实。整体：能用不即插即用，档B。

档 C：不建议或不成熟（本轮已核实，维持不建议）
- 图标类（Iconify）：Iconify 官方组织无 MCP 实现；市面是一批第三方薄封装，最完整的 imjac0b/iconify-mcp-server（`claude mcp add iconify -- npx -y iconify-mcp-server@latest`）也只有 12 star、2025-11 后停更 8 个月、GPL-3.0，本质是 `api.iconify.design` 公开只读接口的薄代理。Claude 自带 WebFetch 可直接查该 API（如 `https://api.iconify.design/search?query=xxx`）拿到同样数据，无需为此多接一个第三方进程。Aurora 当前内联细线 SVG 够用，机械检索出的混杂图标仍需人工对齐线宽，不建议专接。
- 灵感画廊（Dribbble/Behance/Mobbin/Awwwards 聚合）：核实后维持不建议。唯一匹配的多平台聚合实现 YonasValentin/design-inspiration-mcp-server 是"建一次就搁置"的个人项目（3 次提交集中在 2026-03-01 同一天、9 star、无 npm 包、靠第三方 Serper 搜索代理间接检索，易随平台改版失效）。唯一查实的官方实现是 Mobbin 官方 MCP（`claude mcp add mobbin --scope user --transport http https://api.mobbin.com/mcp`，活跃且支持 Claude Code），但只覆盖 Mobbin 一家、且要求现有付费订阅；若团队只需 Mobbin 真机 UI 参考并愿付费，可单独评估。多平台聚合仍建议用 Playwright 抓公开页替代。
- 独立"配色/设计令牌 MCP"：核实后维持不成熟结论。逐一核了 4 个候选（@colorsandfonts/mcp 源码仓库 404、color-palette-mcp/color-scheme-mcp 均个位数 star 个人练手、tailwindcss-mcp-server 为第三方且调色只是杂项功能之一），MCP 官方参考仓库也无相关条目。配色与令牌继续用 `theme.css` 手工维护 + 让主对话直接算色阶，最可控。

## 三、按 Aurora 工作流串联
设计 -> React 组件 -> Tauri 的链路里，这些工具各就各位：
- "看得见"用 Playwright MCP（必要时 Chrome DevTools MCP 补深调）：`pnpm tauri dev`（或 `pnpm dev` 起 Vite）后，让 AI 截图 `localhost:1420`，据实际渲染改 CSS/组件 —— 把 iced 时代"盲调"痛点彻底翻篇。注意应指向 Vite dev server 的浏览器页面而非 Tauri 桌面窗口（后者的 WebView2 不在这两个 MCP 的官方保证范围内）。
- "生成雏形"用 21st Magic 或 shadcn 官方 MCP：描述要的组件（如"账户卡片，纯白，选中态蓝粉渐变描边"），拿到雏形后严格按 `docs/ui-visual-spec.md` 与 `theme.css` token 改写，禁止把它的 Tailwind/CVA/配色原样带入。接入前先拍板是否引入 Tailwind/shadcn。
- "3D 出彩"用 Blender：渲出的图作为静态资源塞进 WebView（png/webp）或做成序列帧/视频背景。
- "设计稿驱动"用 Figma（可选）：若走 Figma，先把蓝粉渐变、间距/圆角/字号刻度建成 Figma Variables，再用官方 Figma MCP 或 Framelink 同步成 CSS 变量，保证一处改动全局一致。

## 四、创新方向（结合工具的具体点子）
1. 3D 极光/方块 hero（Blender MCP）：用 Blender 生成低多边形极光或漂浮 MC 方块群，渲成透明底序列帧作主页顶部背景，呼应"极光/Aurora"命名与"华丽游戏化"。代价：渲染与体积，需控制帧数。
2. 角色/皮肤 3D 预览（Blender MCP）：账户页把玩家皮肤贴到 3D 人物模型上渲染，替代平面头像，做成可缓慢旋转的预览。代价：需要皮肤贴图管线。
3. 视觉自省闭环（Playwright MCP）：建立"改代码 -> Playwright 截图 -> AI 自查是否符合 spec -> 再改"的自动回路，把设计一致性从人肉盯变成可迭代闭环。收益大、代价低，优先做。
4. 组件灵感注入（21st Magic / shadcn MCP）：对每个新页面先生成 2-3 个不同风格雏形，择优再精修，避免单一思路；产物一律回落到我们的 token 体系。
5. 参考站"取其形不取其色"（Playwright）：截 PCL2/麦麦/优秀启动器页面，让 AI 学其布局与信息层级，再用我们的纯白+渐变体系重画，既借鉴又不撞脸。

## 五、落地前的最后一公里（核实已完成，剩下这些要本机验一次）
- 版本门槛：本项目 CLAUDE.md 基线是 Claude Code v2.1.154+，正是"选项须在服务器名之前"的新解析行为区间（21st Magic 的命令修正即因此而来）。接入前 `claude --version` 确认达标。
- 接任何 MCP 都是修改持久化配置，属需授权动作：请由你本人执行上面的 `claude mcp add` / `claude plugin install`，或明确让我代跑；接入后用一个最小调用（如 Playwright 截 `localhost:1420`、Figma 读一个变量）验证连通再投入。
- 仍需一次本机冒烟的点：Playwright 首次会下浏览器内核（确认磁盘/网络 OK）；shadcn 的 `.mcp.json` 记得手补 `"type":"stdio"`；文生图/Framelink/21st 需先备好各自的 API Token/PAT。
- 尚未逐条核实、若要换选型需重走流程的项：Unsplash 的 @jeffkit 替代包、Iconify 的其它第三方实现、fal.ai 相对 Replicate 的 A/B。

---

## 附：工具调用故障与核实方法论（给新会话/我自己）
1. 工具调用被渲染成纯文本、开头混入游离 "课"/"course" 字符：根因是助手在"中文散文 -> 工具调用"边界处偶发插入杂 token，污染了调用起始标签，harness 无法解析、当正文打印。规避：工具调用前不要紧贴中文散文尾字，必要时留干净边界；超长内联内容（如 Workflow 脚本）先 Write 到文件再按路径调用，缩小被污染面。本轮 2026-07-15 的核实工作流即按此把脚本落盘后再发起，未再复现该故障。
2. WebFetch 摘要模型的年份幻觉：本轮对抗式复核中，多个 agent 独立踩到同一坑 —— WebFetch 读 GitHub Releases/文档页时，把 2026 年的发布日期摘要成"2024"甚至"July 3, 2024"（相对时间被小模型算错年份），还臆造过"npm dlx"这类不存在的命令。规避铁律：凡涉及版本号、发布日期、安装命令的事实，一律回退到一手数据源核对 —— GitHub REST API 原始 JSON（`api.github.com/repos/...`）、`raw.githubusercontent.com` 原始文件、`registry.npmjs.org/<pkg>` 的 `time`/`dist-tags` 字段，不采信 WebFetch 的二次摘要。本文件所有版本与日期均按此复核。
