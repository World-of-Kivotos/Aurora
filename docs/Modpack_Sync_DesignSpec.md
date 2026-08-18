# 整合包同步 设计规格

World of Kivotos 服务器客户端整合包的分发、增量更新与后台管理。本文是后续所有实现的唯一真源。

涉及三个仓库：

| 仓库 | 角色 |
|---|---|
| Convenient-access | 服务端：modlist 存储、清单生成、后台 API、进服版本门控 |
| World-of-Kivotos_Web | 管理后台：增删 mod、编辑版本、发布与回滚 |
| Aurora | 启动器客户端：拉清单、算差集、执行同步与首次安装 |

## 一、核心思路

**管理的对象是 modlist，不是整合包目录。**

这决定了整个架构。mod 分两类，处理方式完全不同：

| 类别 | 来源 | 存储 | 带宽消耗 |
|---|---|---|---|
| 平台 mod（Modrinth/CurseForge 上有的） | 后台搜索并指定版本 | **只存引用**（project_id + version_id + 官方 CDN URL + sha1） | **零**，客户端直接从平台 CDN 下载 |
| 自研 mod（Wok-Project 等平台上没有的） | 后台上传 jar | 文件传 OSS，库里存 URL + sha1 | 仅自研部分 |

自研 mod 通常只占整包的一小部分，因此实际自有带宽消耗很低。这套思路与 Modrinth `.mrpack` 一致，是整合包分发的成熟做法。

config 文件与自研 mod 同路：后台上传或在线编辑，存 OSS，清单里带 URL 与 policy。

### 非目标（明确不做，避免后续摇摆）

1. **客户端上报**。不收集 modlist，不上报崩溃日志。排障走玩家在群内自行提供日志。附带收益是 Aurora 不收集任何玩家数据，无需隐私告知。
2. **反作弊**。客户端在任何时候都不可信，Aurora 又是 AGPL-3.0 公开仓库，任何人可合法 fork 并修改。真作弊根本不经过本系统，只能靠服务端行为检测，不在本规格范围内。
3. **二进制 delta（bsdiff/xdelta）**。jar 是 deflate 压缩的 zip，改一个类会让整个压缩流错位，patch 体积经常接近原文件。增量在本规格中一律指文件级差集。

## 二、数据模型

存入 ConvenientAccess 既有 SQLite。[DatabaseManager](../../Convenient-access/src/main/java/com/shinoyuki/accesshub/database/DatabaseManager.java) 已有版本化迁移机制（`database_version` 表 + `migrations/migrate_N_to_N+1.sql`),且明确拒绝静默跳步——新表必须走迁移脚本并提升 `CURRENT_VERSION`,不得手工建表。

```sql
-- 整合包版本
CREATE TABLE pack_version (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    version      TEXT    NOT NULL UNIQUE,      -- 如 "2.0.0"
    status       TEXT    NOT NULL,             -- draft | published | archived
    minecraft    TEXT    NOT NULL,             -- "1.20.1"
    loader_kind  TEXT    NOT NULL,             -- "forge"
    loader_ver   TEXT    NOT NULL,             -- "47.4.20"
    note         TEXT,                         -- 更新说明, 展示给玩家
    created_at   INTEGER NOT NULL,
    published_at INTEGER
);

-- 版本内的文件条目
CREATE TABLE pack_entry (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    version_id   INTEGER NOT NULL REFERENCES pack_version(id) ON DELETE CASCADE,
    path         TEXT    NOT NULL,             -- 相对实例根, 正斜杠, 如 "mods/sodium-0.5.3.jar"
    kind         TEXT    NOT NULL,             -- platform | custom
    policy       TEXT    NOT NULL,             -- managed | seeded | optional
    sha1         TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    download_url TEXT    NOT NULL,             -- 平台 CDN 或 OSS 直链
    -- 以下仅 kind=platform 有值, 供后台展示与"检查平台新版"用
    platform     TEXT,                         -- modrinth | curseforge
    project_id   TEXT,
    project_name TEXT,
    version_ext  TEXT,                         -- 平台侧版本 id
    UNIQUE(version_id, path)
);
CREATE INDEX idx_pack_entry_version ON pack_entry(version_id);
```

关键约束：

1. **`status=published` 的版本不可再修改**。要改就从它复制出一个新草稿。清单不可变是客户端缓存与回滚正确性的前提。
2. **同一时刻至多一个 `published`**，其余已发布过的转 `archived`。当前版本由此唯一确定，不需要额外的指针表。
3. `UNIQUE(version_id, path)` 防止同一版本里两个条目写同一个路径。

## 三、清单格式

客户端拉取的清单由 ConvenientAccess 从上表动态生成。

### 3.1 当前版本指针

`GET /api/v1/pack/latest`（公开，无需鉴权）

```json
{
  "pack_id": "wok",
  "version": "2.0.0",
  "manifest_url": "https://api.mcwok.cn/api/v1/pack/manifest/2.0.0",
  "released_at": "2026-08-17T12:00:00Z",
  "note": "新周目：矿洞维度重做",
  "min_launcher_version": "0.3.0"
}
```

`min_launcher_version` 用于强制启动器升级：清单格式将来若不兼容变更，老启动器应拒绝解析而不是误操作玩家文件。

### 3.2 清单本体

`GET /api/v1/pack/manifest/{version}`（公开）

```json
{
  "schema": 1,
  "pack_id": "wok",
  "version": "2.0.0",
  "minecraft": "1.20.1",
  "loader": { "kind": "forge", "version": "47.4.20" },
  "files": [
    {
      "path": "mods/sodium-0.5.3.jar",
      "sha1": "aa11...",
      "size": 1048576,
      "policy": "managed",
      "urls": ["https://cdn.modrinth.com/data/AANobbMI/versions/xxx/sodium-0.5.3.jar"]
    },
    {
      "path": "mods/wok-core-2.0.0.jar",
      "sha1": "3f2a...",
      "size": 8421376,
      "policy": "managed",
      "urls": ["https://<oss>/wok/files/3f/2a/3f2a..."]
    },
    {
      "path": "options.txt",
      "sha1": "bb22...",
      "size": 2048,
      "policy": "seeded",
      "urls": ["https://<oss>/wok/files/bb/22/bb22..."]
    }
  ]
}
```

要点：

1. **`path` 必须做安全校验**。客户端解析时拒绝 `..`、绝对路径、盘符与 Windows 保留名（`CON`/`PRN`/`AUX`/`NUL`/`COM1..9`/`LPT1..9`）；服务端在写入 `pack_entry` 时同样校验。这是安全边界，两侧都不能省——一份被篡改的清单可以写穿整个磁盘。
2. **自研文件走内容寻址**：OSS 上的路径为 `files/<sha1前2位>/<sha1第3-4位>/<sha1>`。天然跨版本去重，回滚到旧版本时 OSS 上旧文件仍在、客户端本地多半也还留着，回滚几乎零下载。
3. **`sha1` 是唯一的完整性依据**，`size` 仅用于进度预估。
4. 已发布版本的清单不可变，客户端与 CDN 均可长期缓存。

### 3.3 可用性

清单由 MC 进程内的 Jetty 提供，MC 停机时不可用。这是可接受的，因为：

- 已装好的玩家不需要清单也能进游戏（只是查不到更新）。
- 客户端缓存上次成功获取的清单，服务器短暂不可用不影响任何已有玩家。
- 首次安装的新玩家在服务器不可用时装好了也进不去服，等待即可。

因此**不做 OSS 清单静态备份**——它带来的复杂度（双写、一致性）远大于收益。

## 四、文件策略四态

`policy` 决定同步器如何对待一个文件。这是整个设计里最容易做错、也最影响玩家体感的部分。

| policy | 语义 | 本地被修改时 | 从清单移除时 | 典型对象 |
|---|---|---|---|---|
| `managed` | 服务端权威 | 还原 | 删除 | `mods/*.jar`、服务器一致性相关 config、kubejs |
| `seeded` | 首次投递，之后归玩家 | **保留玩家版本** | 保留 | `options.txt`、按键绑定、画质设置 |
| `optional` | 玩家可选装 | 保留 | 保留 | 光影、资源包 |

第四态 `ignore`（`saves/`、`screenshots/`、`logs/`）不需要入库——不在清单里的路径同步器本来就不碰，无须显式声明。

### 硬红线

**删除操作只允许发生在 `managed` 域内，且只允许删除本地快照中记录过的文件。**

必须在代码层面硬约束（删除函数只接受来自快照且 policy 为 managed 的条目），不能靠后台操作时小心。否则一次误操作就能删光玩家存档，这类事故不可逆。

`seeded` 的意义：整合包最经典的差评来源就是每次更新把玩家的按键绑定和画质重置一遍。首次安装写入、之后永不覆盖，是唯一正确的处理。

后台新增条目时按路径给出默认 policy 建议（`mods/**` → managed，`options.txt` → seeded），但最终以人工选择为准。

## 五、三方 diff 语义

### 5.1 为什么必须三方

只对比"清单 vs 磁盘"是不够的：磁盘上多出来的文件，无法区分"玩家自装的私货"与"上个版本有、这版本被移除的 mod"。前者删了要挨骂，后者不删会导致版本不一致。

因此本地必须保存**上次成功应用的清单快照**，落盘 `versions/<id>/.aurora/modpack-applied.json`,与 `ledger.json` 并列。沿用 [ledger.rs](../crates/aurora-core/src/ledger.rs) 既有哲学：磁盘是权威，快照只是索引。

### 5.2 判定表

对每个候选路径，取三个事实：远端清单 `R`、本地快照 `S`、磁盘实际 `D`（含实测 sha1）。

| R | S | D | 判定 |
|---|---|---|---|
| 有 | 无 | 无 | 新增，下载 |
| 有 | 无 | 有且 sha1 相同 | 收编：仅写入快照，不下载 |
| 有 | 无 | 有且 sha1 不同 | managed 覆盖；seeded/optional 保留玩家文件并写快照 |
| 有 | 有 | sha1 == R | 无操作 |
| 有 | 有 | sha1 != R | managed 覆盖；seeded/optional 保留 |
| 有 | 有 | 缺失 | 重新下载（玩家删了或杀软误删） |
| 无 | 有 | 有 | **删除**（仅 managed；其余仅从快照移除） |
| 无 | 有 | 无 | 仅从快照移除 |
| 无 | 无 | 有 | 玩家私货，不碰 |

"收编"一行的意义：玩家从别处拷了一份完全相同的整合包过来时，sha1 已经对上，不该重下一遍。

### 5.3 首次安装就是空快照的同步

**首次全量安装与增量更新走完全相同的代码路径**，区别只在于本地快照为空——此时判定表第一行对所有文件生效，全部下载。不存在"全量模式"与"增量模式"两套逻辑，这是设计上刻意的：两套逻辑意味着两倍的 bug 面，且全量路径平时跑不到、真出问题时才暴露。

首次安装的完整流程（其余步骤复用 Aurora 既有能力）：

1. 拉 `/pack/latest` 与清单
2. 按 `minecraft` + `loader` 安装原版与加载器 —— 走既有 `aurora-install`,不重复实现
3. 创建实例并写入订阅信息
4. 执行同步（空快照 → 全量下载）
5. 写入快照

### 5.4 执行顺序与中断安全

1. 计算差集，得到 `to_download` / `to_delete` / `to_keep`
2. **先下载，全部成功后再删除**。反序会在下载失败时留下残缺实例
3. 全部就位后原子写入新快照
4. 快照写入失败视为整次同步失败——快照与磁盘不一致比没同步更危险

下载走 [aurora-download](../crates/aurora-download/src/) 既有引擎，天然满足：目标已存在且 sha1 符合直接跳过、分片级断点续传、合并后强制校验、多源回退。因此同步可在任意时刻中断，下次从头跑一遍即可收敛，无需事务日志。

## 六、ConvenientAccess 侧实施

### 6.1 API 端点

[ApiRouter](../../Convenient-access/src/main/java/com/shinoyuki/accesshub/api/ApiRouter.java) 是手写 if/else 路径路由 + Bearer JWT 鉴权，第 114-120 行有公开路径白名单。新增：

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET | `/api/v1/pack/latest` | **公开** | 当前发布版本指针 |
| GET | `/api/v1/pack/manifest/{version}` | **公开** | 清单本体 |
| GET | `/api/v1/pack/versions` | 管理员 | 版本列表 |
| POST | `/api/v1/pack/versions` | 管理员 | 新建草稿（可从现有版本复制） |
| GET | `/api/v1/pack/versions/{id}/entries` | 管理员 | 草稿内条目列表 |
| POST | `/api/v1/pack/versions/{id}/entries` | 管理员 | 添加条目（平台引用） |
| PUT | `/api/v1/pack/entries/{id}` | 管理员 | 改 policy / 换版本 |
| DELETE | `/api/v1/pack/entries/{id}` | 管理员 | 移除条目 |
| POST | `/api/v1/pack/versions/{id}/upload` | 管理员 | 上传自研 mod / config（multipart） |
| GET | `/api/v1/pack/versions/{id}/diff` | 管理员 | 与当前发布版比对 |
| POST | `/api/v1/pack/versions/{id}/publish` | 管理员 | 发布 |
| POST | `/api/v1/pack/versions/{id}/rollback` | 管理员 | 回滚到该版本 |

前两个必须加入公开白名单：客户端在玩家登录游戏之前就要拉清单，那时没有任何令牌。

### 6.2 文件上传

HTTP 栈是 [Jetty 11 + jakarta.servlet](../../Convenient-access/src/main/java/com/shinoyuki/accesshub/http/HttpServer.java),`HttpServletRequest.getParts()` 原生支持 multipart，不需要引入额外库。需要在 `ApiHandler` 上配置 `MultipartConfigElement`（临时目录、单文件上限、总请求上限）。

流程：接收 → 落临时文件 → 算 sha1 → 传 OSS（内容寻址路径）→ 写 `pack_entry`。

上限建议：单文件 200 MB，超出拒绝并给出明确文案。自研 mod 不该有这么大，这个上限是防误操作而非防滥用。

### 6.3 OSS 上传：手写签名，不引入 SDK

**不要引入 aliyun-sdk-oss。** 该 SDK 传递依赖较重（jackson、httpclient、jdom 等），而这个 mod 的 shade + relocate 体系已经为 Jetty 与 sqlite 踩过一堆坑（sqlite 因按包路径加载原生库不能 relocate、module-info 必须 exclude、Jetty 关服期惰性加载会崩关服）。为一个 PUT 请求引入重型 SDK 不划算。

OSS 的 PUT Object 只需要：HTTP PUT + `Authorization: OSS <AccessKeyId>:<Signature>`,签名为 HMAC-SHA1 后 Base64。MC 1.20.1 运行在 Java 17，`java.net.http.HttpClient` 与 `javax.crypto.Mac` 都是 JDK 自带，**零新增依赖**。

实施时必须查阅阿里云 OSS 官方签名文档确认 CanonicalizedResource 与头部拼接规则，不得凭记忆编写签名算法。

AccessKey 存配置文件（`AccessHubConfigImpl`),不得进入代码库。建议使用只对该 bucket 有写权限的 RAM 子账号。

### 6.4 平台 mod 的搜索由前端直连

Modrinth API 无需 API key 且支持 CORS，**前端直接调用**,ConvenientAccess 不做代理。后台只在用户选定版本后，把 project_id / version_id / 文件名 / sha1 / 下载 URL 提交入库。

CurseForge 需要 API key，不能放前端。若确需支持，再由 ConvenientAccess 加一个搜索代理端点。**建议第一版只支持 Modrinth**——绝大多数常用 mod 都在，CF 按需再加。

### 6.5 版本门控

挂进既有握手，不新造：

- [PlayerLoginListener](../../Convenient-access/src/main/java/com/shinoyuki/accesshub/event/PlayerLoginListener.java) 的 `PlayerNegotiationEvent` + `disconnectDuringLogin` 已是"进服前拒绝并带文案"的正确实现，线程与时序的坑已踩完。
- [C2SHello](../../Convenient-access/src/main/java/com/shinoyuki/accesshub/deviceauth/net/C2SHello.java) 已解决"客户端装没装本 mod"的协商时序问题。

做法：在同一通道增加携带 `pack_version` 的包（或给 `C2SHello` 加载荷，注意其注释所述兼容约定：老客户端不发也不收，新客户端连老服务端只记一条 invalid discriminator）。服务端**直接查数据库当前 `published` 版本**比对，不需要在配置里手写版本号——发布即生效，不会忘记同步。

配置项：门控开关、拒绝文案。

**门控必须可一键关闭**。发布当晚若清单出问题，你需要能立刻放行所有人，而不是眼睁睁看着全服进不来。

### 门控的信任边界

版本号由客户端自报，可以伪造。这是可接受的：门控的目的是防止玩家因忘记更新而遇到莫名其妙的报错，不是防止恶意进入。刻意伪造版本号硬进的玩家，其后果（物品/方块不同步导致的崩溃）自己承担。

## 七、管理后台

[World-of-Kivotos_Web](../../World-of-Kivotos_Web) 新增"整合包管理"页，复用既有 `@/lib/axios` 与 JWT，无需新增环境变量（全部走 `VITE_API_BASE_URL`）。

功能：

1. **版本列表**：当前发布版、历史版本、草稿；一键"从此版本复制新草稿"
2. **草稿编辑**：
   - mod 列表（区分平台 mod 与自研上传，展示 policy）
   - 搜索 Modrinth 添加：搜项目 → 选版本（按 MC 版本与加载器过滤）→ 加入
   - 上传自研 mod / config
   - 改 policy、移除条目
3. **发布前 diff**：与当前发布版比对，列出新增/变更/删除。**删除项需要显式确认**——尤其当删除数量异常时，那几乎一定是操作失误
4. **发布 / 回滚**

发布是不可逆的对外操作（数百玩家会立刻开始同步），必须二次确认。

## 八、Aurora 侧实施

### 8.1 受管实例

实例可订阅一个整合包，落盘 `versions/<id>/.aurora/modpack-subscription.json`：

```json
{ "pack_id": "wok", "pointer_url": "https://api.mcwok.cn/api/v1/pack/latest" }
```

**受管实例必须禁用平台更新检查**。[updates.rs](../crates/aurora-core/src/updates.rs) 的 `check_updates` 干的是"每个 mod 各自查 Modrinth/CurseForge 最新版",对服务器整合包不仅无意义而且有害：玩家各自更新到最新版即版本不一致，进不去服。在受管实例上应直接返回空并在 UI 上说明原因。

### 8.2 新增 crate `aurora-modpack`

放在 `aurora-modplatform` 同层，依赖 `aurora-base` + `aurora-download`。职责：

- 清单与指针的模型定义、解析、路径安全校验
- 快照读写
- 三方 diff（**纯函数，无 IO，可完整单测**）
- 同步计划执行（调用 `DownloadPool`）

diff 是纯函数这一点很重要：它是全系统最容易造成不可逆损失（误删）的地方，必须用表驱动测试覆盖 5.2 的每一行判定。

### 8.3 MirrorSource 需要扩展

[source.rs](../crates/aurora-download/src/source.rs) 的 `SourceResolver` 是可注入 trait，但第二个参数 `MirrorSource` 是闭合枚举（仅 `Official` / `BmclApi`）。整合包的多候选 URL 需要新增变体表达。

改动方式：给枚举加变体，让 Rust 的穷尽 match 把所有需要处理的点报出来。**不要硬塞进 `BmclApi` 变体**——语义错误会在半年后变成没人看得懂的 bug。

### 8.4 UI

- 一键安装：输入/内置服务器整合包地址 → 装 MC + 加载器 → 同步 → 完成
- 已装实例：显示当前版本与可用版本、更新按钮、进度与失败详情
- mod 列表区分"整合包提供"与"玩家自装",前者不给删除按钮（删了下次同步会还原，给按钮只会造成困惑）

同步失败必须给出可操作文案：哪个文件失败、失败原因（网络/校验不符/磁盘满），而不是一句"同步失败"。

## 九、实施顺序

| 阶段 | 内容 | 依赖 |
|---|---|---|
| 1 | `aurora-modpack`:清单模型、路径安全校验、快照、三方 diff（纯逻辑 + 完整单测） | 无 |
| 2 | ConvenientAccess:迁移脚本、两张表、清单生成与公开端点 | 无 |
| 3 | ConvenientAccess:管理端 CRUD、OSS 上传、diff、发布/回滚 | 阶段 2 |
| 4 | Aurora 同步执行：受管实例、禁用平台更新检查、`MirrorSource` 扩展 | 阶段 1 |
| 5 | Web 后台整合包管理页 | 阶段 3 |
| 6 | Aurora UI：一键安装与更新 | 阶段 4 |
| 7 | 版本门控握手（双端） | 阶段 2、4 |

阶段 1 与 2 无相互依赖，可并行。阶段 1 是全系统风险最高的部分（误删不可逆），应先于一切完成并测透。

## 十、待确认事项

1. **OSS 厂商与 bucket**。建议为其单开只有该 bucket 写权限的 RAM 子账号。
2. **是否支持 CurseForge**。第一版建议只做 Modrinth（前端直连、无需 key）。
3. **config 的管理方式**。当前设计为与自研 mod 同路（上传文件）。若需要在线编辑文本，可后续加一个文本编辑端点，不影响数据模型。
