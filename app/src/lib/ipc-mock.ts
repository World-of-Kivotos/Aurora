// 浏览器 mock：不在 Tauri 环境时（如 `pnpm dev` 用 puppeteer/浏览器看 UI），用假数据驱动全部页面，
// 让前端能脱离 Rust 后端独立开发/截图。仅开发期生效——正式打包在 Tauri 里走真 IPC（见 tauri-bridge.ts）。

import type { UnlistenFn } from "@tauri-apps/api/event";
// Mod 生态那批 mock 数据直接用 ipc.ts 的 DTO 类型标注：一旦 mock 与契约漂移，tsc 当场报错，
// 而不是等接上真后端才在运行期发现字段名对不上。仅类型导入，不产生运行期循环依赖。
import type {
  CrashDiagnosis,
  CrashReport,
  DeviceCode,
  GlassMode,
  History,
  HistoryEvent,
  InstallPlan,
  InstanceMatch,
  Ledger,
  LedgerEntry,
  ManagedModpackFile,
  ModVersionInfo,
  PlatformId,
  RollbackCheck,
  UpdateCandidate,
} from "./ipc";
import type { CheckedManagedModpackStatus, ModpackSyncError } from "./modpack-ui";

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface MockAccount {
  uuid: string;
  name: string;
  account_type: "microsoft" | "offline" | "authlib_injector";
}

// 账户在 mock 里是有状态的：离线账户后端已改成持久化（明文 offline_accounts.json），
// 浏览器里也必须能增/删/切换后立刻在列表上看到结果，否则这一页在 dev 模式下看着永远是死的。
const ACCOUNTS: MockAccount[] = [
  { uuid: "853c80ef3c3749fdaa49938b674adae6", name: "Shinoyuki_Miyako", account_type: "microsoft" },
  // uuid 走与 create_offline_account 同一个派生函数，重复添加 Steve 才会如实表现成幂等。
  { uuid: mockOfflineUuid("Steve"), name: "Steve", account_type: "offline" },
];

let currentAccountUuid: string | null = ACCOUNTS[0].uuid;

// 浏览器里没有 md5，造不出与原版一致的离线 UUID；mock 只需保证「同名恒同 uuid」这一条
// 可观测性质（头像与「当前」判定都吃它），故用 FNV-1a 摊成 32 位十六进制顶替。
function mockOfflineUuid(name: string): string {
  let hash = 0x811c9dc5;
  for (const ch of name) {
    hash = Math.imul(hash ^ ch.codePointAt(0)!, 0x01000193) >>> 0;
  }
  return Array.from({ length: 4 }, (_, i) =>
    (Math.imul(hash + i * 0x9e3779b9, 0x85ebca6b) >>> 0).toString(16).padStart(8, "0"),
  ).join("");
}

// 保存离线账户：同名幂等，且照后端语义把它设为当前账户。
function addOfflineAccount(name: string): MockAccount {
  // 三条硬性规则照抄后端 validate_username，好让浏览器里也能看到真实的报错文案。
  if (!name) throw new Error("离线用户名不合法: 用户名不能为空");
  if (name.includes('"')) throw new Error("离线用户名不合法: 用户名不能包含双引号");
  if (Array.from(name).length > 16) {
    throw new Error("离线用户名不合法: 用户名不能超过 16 个字符（1.20.3 及以上版本限制）");
  }
  const account: MockAccount = {
    uuid: mockOfflineUuid(name),
    name,
    account_type: "offline",
  };
  const existing = ACCOUNTS.find((a) => a.uuid === account.uuid);
  if (!existing) ACCOUNTS.push(account);
  currentAccountUuid = account.uuid;
  return existing ?? account;
}

function removeMockAccount(uuid: string): void {
  const index = ACCOUNTS.findIndex((a) => a.uuid === uuid);
  if (index < 0) throw new Error(`账户不存在: ${uuid}`);
  ACCOUNTS.splice(index, 1);
  // 删掉的正是当前账户时回落到剩余第一个，与后端两个库的删除语义一致。
  if (currentAccountUuid === uuid) currentAccountUuid = ACCOUNTS[0]?.uuid ?? null;
}

function setMockCurrentAccount(uuid: string): void {
  if (!ACCOUNTS.some((a) => a.uuid === uuid)) throw new Error(`账户不存在: ${uuid}`);
  currentAccountUuid = uuid;
}

// 在线登录：账户被删掉后还能再登回来，登录成功即成为当前账户（与后端 upsert 一致）。
function loginMockAccount(account: MockAccount): MockAccount {
  if (!ACCOUNTS.some((a) => a.uuid === account.uuid)) ACCOUNTS.push(account);
  currentAccountUuid = account.uuid;
  return account;
}

// 设备码事件的订阅表。微软登录在 mock 里也走真后端那套两段式（先推配对码、再等轮询完成），
// 否则浏览器里那个登录弹窗永远停在「正在向微软申请配对码」，180ms 后直接登录成功——
// 弹窗的排版、让位与「打开验证网址」那颗键在 dev 模式下根本没机会露面，验不了。
const deviceCodeHandlers = new Set<(code: DeviceCode) => void>();

// 配对码停留时长：短到不至于让人干等，长到看得清那串码与那行网址。真登录是玩家去网页上输码，
// 时长由玩家自己决定；mock 只需把这一段「有码可看」的时间做出来。
const MOCK_DEVICE_CODE_MS = 2600;

// 微软登录：推一帧假配对码给订阅者，停留一会儿再落账户，与后端 microsoft_login 的时序同构。
async function microsoftLoginMock(): Promise<MockAccount> {
  const code: DeviceCode = {
    user_code: "AURORA-DEV",
    verification_uri: "https://www.microsoft.com/link",
    expires_in: 900,
    interval: 5,
    message: "浏览器 mock 的假配对码：真登录只在 Tauri 里走，这里只为把弹窗的样子做出来。",
  };
  deviceCodeHandlers.forEach((notify) => notify(code));
  await delay(MOCK_DEVICE_CODE_MS);
  return loginMockAccount(MICROSOFT_SEED);
}

const MICROSOFT_SEED: MockAccount = {
  uuid: "853c80ef3c3749fdaa49938b674adae6",
  name: "Shinoyuki_Miyako",
  account_type: "microsoft",
};

const AUTHLIB_SEED: MockAccount = {
  uuid: "1f5c3a7d9b2e4c6a8d0f2b4c6e8a0d2f",
  name: "LittleSkin_Player",
  account_type: "authlib_injector",
};

const INSTALLED = {
  versions: [
    {
      id: "World of Kivotos 2.0 beta",
      mc_version: "1.20.1",
      is_release: true,
      has_mod_loader: true,
      loaders: [{ kind: "forge", version: "47.4.16" }],
    },
    {
      id: "1.20.1-Forge_47.4.20",
      mc_version: "1.20.1",
      is_release: true,
      has_mod_loader: true,
      loaders: [{ kind: "forge", version: "47.4.20" }],
    },
    {
      id: "1.21.1-Fabric",
      mc_version: "1.21.1",
      is_release: true,
      has_mod_loader: true,
      loaders: [{ kind: "fabric", version: "0.16.5" }],
    },
    { id: "1.21.4", mc_version: "1.21.4", is_release: true, has_mod_loader: false, loaders: [] },
    { id: "测试服", mc_version: "1.20.1", is_release: true, has_mod_loader: true, loaders: [{ kind: "forge", version: "47.4.16" }] },
  ],
  broken: [{ id: "24w14potato", reason: "版本 JSON 损坏：unexpected end of input" }],
};

const CONFIG = {
  game_dir: "D:\\PCL2\\.minecraft",
  data_dir: "C:\\Users\\Xiaoxiao\\AppData\\Local\\Aurora",
  download_source: "auto",
  version_list_source: "auto",
  download_concurrency: 64,
  memory: { max_mb: 8192, min_mb: null },
  isolation_policy: "mod_loaders_and_non_release",
  has_client_id: true,
  auto_download_java: true,
  selected_version: "World of Kivotos 2.0 beta",
};

const MOCK_MANAGED_VERSION_ID = "World of Kivotos 2.0 beta";

const MOCK_MANAGED_STATUS: CheckedManagedModpackStatus = {
  kind: "ready",
  subscription: {
    pack_id: "wok-browser-preview",
    pointer_url: "https://mock.invalid/api/v1/pack/latest",
  },
  versions: {
    installed_version: "1.9.0-preview",
    latest: {
      pack_id: "wok-browser-preview",
      version: "2.0.0-preview",
      manifest_url: "https://mock.invalid/api/v1/pack/manifest/2.0.0-preview",
      released_at: "2026-08-17T12:00:00Z",
      note: "浏览器模拟状态，不代表磁盘上存在真实整合包或文件。",
      min_launcher_version: "0.1.0",
    },
  },
  source: "cache",
  checked_at: "1786968300",
};

const MOCK_MANAGED_FILES: ManagedModpackFile[] = [
  { path: "mods/sodium-fabric-0.6.0.jar", policy: "managed" },
  { path: "config/wok-client.toml", policy: "seeded" },
];

function browserPreviewWriteError(targetVersion: string): ModpackSyncError {
  return {
    target_version: targetVersion,
    stage: "resolving_manifest",
    failure: {
      kind: "filesystem",
      file_path: "<browser-preview>",
      detail: "浏览器预览只模拟界面状态，不会执行真实安装或写入磁盘。请在 Tauri 应用中运行此操作。",
    },
  };
}

// 搜索结果：Modrinth v2 search 各类型下载量 Top 8 的真实快照（标题/作者/下载量/图标 URL/分类均为线上原值）。
// 用真图标而非造假数据，是为了让卡片排版在开发期就吃到真实的图片比例、长标题与中英混排。
const SEARCH_HITS = [
  { platform: "modrinth", project_id: "P7dR8mSH", slug: "fabric-api", title: "Fabric API", description: "Lightweight and modular API providing common hooks and intercompatibility measures utilized by mods using the Fabric toolchain.", author: "modmuss50", downloads: 218114806, follows: 34470, icon_url: "https://cdn.modrinth.com/data/P7dR8mSH/icon.png", categories: ["fabric","library"], resource_type: "mod", date_modified: "2026-07-28", page_url: "https://modrinth.com/mod/fabric-api" },
  { platform: "modrinth", project_id: "AANobbMI", slug: "sodium", title: "Sodium", description: "A high-performance rendering engine replacement for Minecraft, which greatly improves frame rates and reduces micro-stutter.", author: "jellysquid3", downloads: 195391958, follows: 39228, icon_url: "https://cdn.modrinth.com/data/AANobbMI/295862f4724dc3f78df3447ad6072b2dcd3ef0c9_96.webp", categories: ["fabric","neoforge","optimization"], resource_type: "mod", date_modified: "2026-07-08", page_url: "https://modrinth.com/mod/sodium" },
  { platform: "curseforge", project_id: "YL57xq9U", slug: "iris", title: "Iris Shaders", description: "A modern shader pack loader for Minecraft intended to be compatible with existing OptiFine shader packs", author: "coderbot", downloads: 152359588, follows: 28208, icon_url: "https://cdn.modrinth.com/data/YL57xq9U/18d0e7f076d3d6ed5bedd472b853909aac5da202_96.webp", categories: ["decoration","fabric","neoforge"], resource_type: "mod", date_modified: "2026-07-09", page_url: "https://modrinth.com/mod/iris" },
  { platform: "modrinth", project_id: "9s6osm5g", slug: "cloth-config", title: "Cloth Config API", description: "Configuration Library for Minecraft Mods", author: "shedaniel", downloads: 146852356, follows: 16092, icon_url: "https://cdn.modrinth.com/data/9s6osm5g/ed8a2316cbb6f4fc5f510e8e13a59a85cbbbff4d_96.webp", categories: ["fabric","forge","library"], resource_type: "mod", date_modified: "2026-06-18", page_url: "https://modrinth.com/mod/cloth-config" },
  { platform: "modrinth", project_id: "NNAgCjsB", slug: "entityculling", title: "Entity Culling", description: "Using async path-tracing to hide Block-/Entities that are not visible", author: "tr7zw", downloads: 142507344, follows: 16624, icon_url: "https://cdn.modrinth.com/data/NNAgCjsB/7873452d6cede4daed12da3d7d8c193ab88b4fd6_96.webp", categories: ["babric","fabric","forge"], resource_type: "mod", date_modified: "2026-06-20", page_url: "https://modrinth.com/mod/entityculling" },
  { platform: "curseforge", project_id: "uXXizFIs", slug: "ferrite-core", title: "FerriteCore", description: "Memory usage optimizations", author: "malte0811", downloads: 133090982, follows: 15526, icon_url: "https://cdn.modrinth.com/data/uXXizFIs/222a126f26f8f9ae1eb339f3b767677f18bff31f_96.webp", categories: ["fabric","forge","neoforge"], resource_type: "mod", date_modified: "2026-03-24", page_url: "https://modrinth.com/mod/ferrite-core" },
  { platform: "modrinth", project_id: "mOgUt4GM", slug: "modmenu", title: "Mod Menu", description: "Adds a mod menu to view the list of mods you have installed.", author: "Prospector", downloads: 126149575, follows: 26077, icon_url: "https://cdn.modrinth.com/data/mOgUt4GM/5a20ed1450a0e1e79a1fe04e61bb4e5878bf1d20.png", categories: ["fabric","quilt","utility"], resource_type: "mod", date_modified: "2026-07-23", page_url: "https://modrinth.com/mod/modmenu" },
  { platform: "modrinth", project_id: "gvQqBUqZ", slug: "lithium", title: "Lithium", description: "No-compromises game logic optimization mod, useful for both single-player games and multi-player servers.", author: "jellysquid3", downloads: 112760637, follows: 22750, icon_url: "https://cdn.modrinth.com/data/gvQqBUqZ/bcc8686c13af0143adf4285d741256af824f70b7_96.webp", categories: ["fabric","neoforge","optimization"], resource_type: "mod", date_modified: "2026-07-29", page_url: "https://modrinth.com/mod/lithium" },
  { platform: "curseforge", project_id: "1KVo5zza", slug: "fabulously-optimized", title: "Fabulously Optimized", description: "Beautiful graphics, speedy performance and familiar features in a simple package. Chaos Cubed beta!", author: "robotkoer", downloads: 15112993, follows: 4657, icon_url: "https://cdn.modrinth.com/data/1KVo5zza/d8152911f8fd5d7e9a8c499fe89045af81fe816e_96.webp", categories: ["fabric","lightweight","multiplayer"], resource_type: "modpack", date_modified: "2026-07-26", page_url: "https://modrinth.com/modpack/fabulously-optimized" },
  { platform: "modrinth", project_id: "l9m9tuPN", slug: "zombie-invade-100-days", title: "Zombie Invade 100 Days", description: "僵尸入侵 100 天（高版本惊变重制）—— Same as Forge Labs 100 Days Zombie Apocalypse in new Minecraft", author: "FlameFire", downloads: 12328824, follows: 583, icon_url: "https://cdn.modrinth.com/data/l9m9tuPN/fefe3f67c37744344d100638452c7bf059d586a1_96.webp", categories: ["challenging","combat","forge"], resource_type: "modpack", date_modified: "2025-12-13", page_url: "https://modrinth.com/modpack/zombie-invade-100-days" },
  { platform: "modrinth", project_id: "5FFgwNNP", slug: "cobblemon-fabric", title: "Cobblemon Official Modpack [Fabric]", description: "The official modpack of the Cobblemon mod, for Fabric!", author: "CobbledStudios", downloads: 9177434, follows: 2488, icon_url: "https://cdn.modrinth.com/data/5FFgwNNP/e7f9ee2e9d361623847853fe2ddce42f519ee64f.png", categories: ["adventure","fabric","lightweight"], resource_type: "modpack", date_modified: "2026-01-31", page_url: "https://modrinth.com/modpack/cobblemon-fabric" },
  { platform: "curseforge", project_id: "Jkb29YJU", slug: "cobbleverse", title: "COBBLEVERSE - Pokemon Adventure [Cobblemon]", description: "Start a true Pokémon adventure in Minecraft: Cobblemon 1.7.3 | ALL 1025 Pokémon! | Mega Evolutions | Gyms & Badges | Starter Kit | Unique Structures |", author: "LUMYVERSE", downloads: 5474525, follows: 1749, icon_url: "https://cdn.modrinth.com/data/Jkb29YJU/581ecf54530972afb18a04660afd820f2f24f6c7.png", categories: ["adventure","challenging","fabric"], resource_type: "modpack", date_modified: "2026-07-21", page_url: "https://modrinth.com/modpack/cobbleverse" },
  { platform: "modrinth", project_id: "shFhR8Vx", slug: "better-mc-fabric-bmc2", title: "Better MC [FABRIC] - BMC2", description: "Version 1.20 | A Proper Vanilla+ Modpack | Don't play Vanilla play this!", author: "SHXRKIE", downloads: 2885761, follows: 1332, icon_url: "https://cdn.modrinth.com/data/shFhR8Vx/a19c2bcb51d38f32f138d3607e91cb2b7b8e387f_96.webp", categories: ["adventure","combat","fabric"], resource_type: "modpack", date_modified: "2026-07-15", page_url: "https://modrinth.com/modpack/better-mc-fabric-bmc2" },
  { platform: "modrinth", project_id: "ch7UHY2J", slug: "sodiumplus", title: "Sodium Plus", description: "A client-side optimization modpack with a few extra tweaks.", author: "NoSadBeHappy", downloads: 2382021, follows: 589, icon_url: "https://cdn.modrinth.com/data/ch7UHY2J/cf0150ba8d6c01144974709a27b23bba93c0fc9e.png", categories: ["fabric","lightweight","multiplayer"], resource_type: "modpack", date_modified: "2026-07-20", page_url: "https://modrinth.com/modpack/sodiumplus" },
  { platform: "curseforge", project_id: "rQiXwLhB", slug: "battlearmorytacz", title: "BattleArmory TACZ", description: "厌倦了每次开局都是一样的钻石剑和弓？BattleArmory 让你每场重生都像开盲盒——400+ 现代枪械随机配发，配上经典地图与战术投掷物。", author: "JZ_zhenmeng", downloads: 2261451, follows: 51, icon_url: "https://cdn.modrinth.com/data/rQiXwLhB/1f8cafea7dcd59dff7dc58cad7a17ae0424847a2_96.webp", categories: ["combat","forge","multiplayer"], resource_type: "modpack", date_modified: "2026-06-20", page_url: "https://modrinth.com/modpack/battlearmorytacz" },
  { platform: "modrinth", project_id: "1ocGzRHv", slug: "vanilla-perfected", title: "Vanilla Perfected", description: "A compilation of Vanilla Plus mods & packs to perfect the Minecraft experience without drifting too far from the feel of the base game!", author: "demonjoeTV", downloads: 2165372, follows: 2331, icon_url: "https://cdn.modrinth.com/data/1ocGzRHv/ebec737126db7637001194ca5560ae58413d338f_96.webp", categories: ["adventure","fabric","lightweight"], resource_type: "modpack", date_modified: "2026-07-24", page_url: "https://modrinth.com/modpack/vanilla-perfected" },
  { platform: "modrinth", project_id: "50dA9Sha", slug: "fresh-animations", title: "Fresh Animations", description: "Make your game like the trailers! Dynamic animated entities to freshen your Minecraft experience.", author: "FreshLX", downloads: 40975370, follows: 13527, icon_url: "https://cdn.modrinth.com/data/50dA9Sha/3132c10e9e3c73fde9799720fd3da5561071708c_96.webp", categories: ["16x","entities","minecraft"], resource_type: "resource_pack", date_modified: "2026-04-01", page_url: "https://modrinth.com/resourcepack/fresh-animations" },
  { platform: "curseforge", project_id: "yfDziwn1", slug: "translations-for-sodium", title: "Translations for Sodium", description: "Unofficial translations for the Sodium Minecraft mod", author: "robotkoer", downloads: 18457841, follows: 1503, icon_url: "https://cdn.modrinth.com/data/yfDziwn1/907581019df45903df237952ce8d10ac37134cb5_96.webp", categories: ["locale","minecraft","modded"], resource_type: "resource_pack", date_modified: "2026-07-25", page_url: "https://modrinth.com/resourcepack/translations-for-sodium" },
  { platform: "modrinth", project_id: "uvpymuxq", slug: "better-leaves", title: "Motschen's Better Leaves", description: "Improves the appearance of leaves with high mod compatibility and performance!", author: "Motschen", downloads: 17510358, follows: 4712, icon_url: "https://cdn.modrinth.com/data/uvpymuxq/fe1a61998ae57dc6ad1a4bb028334c3c3925d22f_96.webp", categories: ["minecraft","modded","models"], resource_type: "resource_pack", date_modified: "2026-01-31", page_url: "https://modrinth.com/resourcepack/better-leaves" },
  { platform: "modrinth", project_id: "tN4E9NfV", slug: "chat-reporting-helper", title: "Chat Reporting Helper", description: "An educational tool that explains chat reporting in a simple and neutral way.", author: "robotkoer", downloads: 14487863, follows: 270, icon_url: "https://cdn.modrinth.com/data/tN4E9NfV/c2d93fd85469a512f67e53baa7648c1abe6645ef.png", categories: ["16x","gui","locale"], resource_type: "resource_pack", date_modified: "2026-06-22", page_url: "https://modrinth.com/resourcepack/chat-reporting-helper" },
  { platform: "curseforge", project_id: "slufHzC2", slug: "tras-fresh-player", title: "Fresh Moves", description: "This is EMF Player Animation resource pack. Gives fancy animations to the player entity. Compatible with animations from other mods.", author: "IthanMendoza", downloads: 12791192, follows: 3790, icon_url: "https://cdn.modrinth.com/data/slufHzC2/02b78db6655ad8dd6cfd847f99c76f7552c053ec_96.webp", categories: ["16x","equipment","minecraft"], resource_type: "resource_pack", date_modified: "2025-01-20", page_url: "https://modrinth.com/resourcepack/tras-fresh-player" },
  { platform: "modrinth", project_id: "RRxvWKNC", slug: "low-on-fire", title: "Low On Fire", description: "Low fire on your screen! Vanilla Friendly", author: "Haikis", downloads: 12679977, follows: 3328, icon_url: "https://cdn.modrinth.com/data/RRxvWKNC/06a7e7691a6da41798c255108a8563d7ffea171d.png", categories: ["16x","combat","minecraft"], resource_type: "resource_pack", date_modified: "2026-06-19", page_url: "https://modrinth.com/resourcepack/low-on-fire" },
  { platform: "modrinth", project_id: "dspVZXKP", slug: "fast-better-grass", title: "Fast Better Grass", description: "Makes grass and related blocks use the top texture on the sides. Works with other resource packs!", author: "robotkoer", downloads: 11753407, follows: 2487, icon_url: "https://cdn.modrinth.com/data/dspVZXKP/5de0774f423d220fcd4e635b91f2bdfcb8a2b910.png", categories: ["128x","16x","256x"], resource_type: "resource_pack", date_modified: "2026-06-16", page_url: "https://modrinth.com/resourcepack/fast-better-grass" },
  { platform: "curseforge", project_id: "ItHr72Fy", slug: "fullbright-ub", title: "Fullbright UB", description: "Experience Fullbright UB in Sodium, Optifine and Vanilla", author: "worldresourcepack", downloads: 9803872, follows: 2331, icon_url: "https://cdn.modrinth.com/data/ItHr72Fy/c28c5e4ae4e1e5ac726ff192855d16d2c15c83e2_96.webp", categories: ["core-shaders","environment","minecraft"], resource_type: "resource_pack", date_modified: "2025-10-12", page_url: "https://modrinth.com/resourcepack/fullbright-ub" },
  { platform: "modrinth", project_id: "HVnmMxH1", slug: "complementary-reimagined", title: "Complementary Shaders - Reimagined", description: "Preserving the elements of Minecraft with exceptional quality, detail, and performance.", author: "EminGT", downloads: 58591131, follows: 10323, icon_url: "https://cdn.modrinth.com/data/HVnmMxH1/79cb7c8123bbc54945305b2ebad6b8881efdf5f8_96.webp", categories: ["atmosphere","bloom","cartoon"], resource_type: "shader", date_modified: "2026-05-21", page_url: "https://modrinth.com/shader/complementary-reimagined" },
  { platform: "modrinth", project_id: "R6NEzAwj", slug: "complementary-unbound", title: "Complementary Shaders - Unbound", description: "Transforming the visuals of Minecraft with exceptional quality, detail, and performance.", author: "EminGT", downloads: 38055090, follows: 5541, icon_url: "https://cdn.modrinth.com/data/R6NEzAwj/c85ce4049aac76360d2cd24fd9a7003de01ef312_96.webp", categories: ["atmosphere","bloom","cartoon"], resource_type: "shader", date_modified: "2026-05-21", page_url: "https://modrinth.com/shader/complementary-unbound" },
  { platform: "curseforge", project_id: "Q1vvjJYV", slug: "bsl-shaders", title: "BSL Shaders", description: "Shaderpack for Minecraft: Java Edition. It's bright, colorful, and distinct.", author: "CaptTatsu", downloads: 26097069, follows: 5890, icon_url: "https://cdn.modrinth.com/data/Q1vvjJYV/2a611a3cb434fb52fb81fa5dace13c5d8b67e55d_96.webp", categories: ["atmosphere","bloom","cartoon"], resource_type: "shader", date_modified: "2026-04-20", page_url: "https://modrinth.com/shader/bsl-shaders" },
  { platform: "modrinth", project_id: "lLqFfGNs", slug: "photon-shader", title: "Photon Shaders", description: "A gameplay-focused shader pack with a semi-realistic style", author: "sixthsurge", downloads: 22890620, follows: 3880, icon_url: "https://cdn.modrinth.com/data/lLqFfGNs/39cb5f12e7dcc68d6cb666f225fcb2b801dd70fb_96.webp", categories: ["atmosphere","bloom","colored-lighting"], resource_type: "shader", date_modified: "2026-04-14", page_url: "https://modrinth.com/shader/photon-shader" },
  { platform: "modrinth", project_id: "EpQFjzrQ", slug: "solas-shader", title: "Solas Shader", description: "A modern fantasy stylized shaderpack with colored lighting and stunning visuals", author: "Septonious", downloads: 14681487, follows: 3075, icon_url: "https://cdn.modrinth.com/data/EpQFjzrQ/e3efc6ba7a63f9e1cf473a794d0224a6daf243c7_96.webp", categories: ["atmosphere","bloom","cartoon"], resource_type: "shader", date_modified: "2026-07-01", page_url: "https://modrinth.com/shader/solas-shader" },
  { platform: "curseforge", project_id: "ZvMtQlho", slug: "bliss-shader", title: "Bliss Shaders", description: "A well performing fantasy styled shaderpack with emphasis on scene variation and customization.", author: "Xonk", downloads: 12448079, follows: 2658, icon_url: "https://cdn.modrinth.com/data/ZvMtQlho/90145c971ea24387775108fc86c89bed9bd2c8f1_96.webp", categories: ["atmosphere","bloom","colored-lighting"], resource_type: "shader", date_modified: "2025-11-23", page_url: "https://modrinth.com/shader/bliss-shader" },
  { platform: "modrinth", project_id: "kmwfVOoi", slug: "rethinking-voxels", title: "Rethinking Voxels", description: "[WIP] A gameplay shaderpack based on complementary reimagined that has coloured block light with sharp shadows", author: "gri573", downloads: 11578062, follows: 4361, icon_url: "https://cdn.modrinth.com/data/kmwfVOoi/fc89eadad417dd376b14c3b31e1a2b87acaca034_96.webp", categories: ["atmosphere","bloom","colored-lighting"], resource_type: "shader", date_modified: "2025-06-19", page_url: "https://modrinth.com/shader/rethinking-voxels" },
  { platform: "modrinth", project_id: "izsIPI7a", slug: "makeup-ultra-fast-shaders", title: "MakeUp - Ultra Fast", description: "MakeUp aims to provide the best quality / performance ratio, building a shader that can be adapted to anyone's resources.", author: "KDXavier", downloads: 10977140, follows: 2194, icon_url: "https://cdn.modrinth.com/data/izsIPI7a/a08432baa86b8ffd58c08f4b3a001ef976ff764d_96.webp", categories: ["atmosphere","bloom","fantasy"], resource_type: "shader", date_modified: "2026-06-27", page_url: "https://modrinth.com/shader/makeup-ultra-fast-shaders" },
];

// 原实现对 type==="mod" 直接放行全部条目，导致 Mod 页混进光影、而整合包/资源包页恒空。改为严格按类型过滤。
function searchResult(query: string, type: string, sort: string) {
  const q = (query || "").toLowerCase();
  const hits = SEARCH_HITS.filter((h) => h.resource_type === type).filter(
    (h) => !q || h.title.toLowerCase().includes(q) || h.description.toLowerCase().includes(q),
  );
  const sorted =
    sort === "downloads"
      ? [...hits].sort((a, b) => b.downloads - a.downloads)
      : sort === "updated"
        ? [...hits].sort((a, b) => b.date_modified.localeCompare(a.date_modified))
        : sort === "follows"
          ? [...hits].sort((a, b) => b.follows - a.follows)
          : hits;
  return { hits: sorted, errors: [] };
}

const MODS = [
  { path: "", file_name: "sodium-fabric-0.6.0.jar", enabled: true, metadata: { mod_id: "sodium", name: "Sodium", version: "0.6.0", description: null, authors: ["jellysquid3"], loader: "fabric", format: "fabric.mod.json" } },
  { path: "", file_name: "fabric-api-0.115.0.jar", enabled: true, metadata: { mod_id: "fabric-api", name: "Fabric API", version: "0.115.0", description: null, authors: [], loader: "fabric", format: "fabric.mod.json" } },
  { path: "", file_name: "jei-19.21.0.jar", enabled: false, metadata: { mod_id: "jei", name: "Just Enough Items", version: "19.21.0", description: null, authors: ["mezz"], loader: "fabric", format: "fabric.mod.json" } },
];

// 游戏目录：mock 内存态。默认不当作首次启动——每次热重载都弹一次向导就没法干别的活。
// 要看初次设定，在地址里带上 ?firstRun=1，或在控制台
// localStorage.setItem("aurora.mock.firstRun", "1") 之后刷新。
const FIRST_RUN = {
  pending:
    typeof window !== "undefined" &&
    (window.location.href.includes("firstRun=1") ||
      window.localStorage?.getItem("aurora.mock.firstRun") === "1"),
};

/** 已收下的「其它文件夹」。 */
const EXTRA_DIRS: { name: string; path: string }[] = [
  { name: "官方启动器", path: "C:\\Users\\Xiaoxiao\\AppData\\Roaming\\.minecraft" },
];

/** 探测得到的候选：故意留一条尚未收下的，好让初次设定的推荐列表非空。 */
const DISCOVERABLE: { name: string; path: string }[] = [
  { name: "官方启动器", path: "C:\\Users\\Xiaoxiao\\AppData\\Roaming\\.minecraft" },
  { name: "PCL2", path: "E:\\Plain Craft Launcher 2\\.minecraft" },
  { name: "HMCL", path: "D:\\HMCL\\.minecraft" },
];

// 外观：mock 内存态。默认给一张背景，好让浏览器里直接看到「图 + 纸片」那套版式；
// 想看纯纸面的样子在设置页点「恢复纯纸面」即可。
const BACKGROUNDS: {
  file: string;
  width: number;
  height: number;
  bytes: number;
  is_current: boolean;
}[] = [
  { file: "雪山黄昏.jpg", width: 1920, height: 1080, bytes: 386_512, is_current: true },
  { file: "海岸线.jpg", width: 1920, height: 1280, bytes: 512_904, is_current: false },
  { file: "夜航.jpg", width: 1600, height: 900, bytes: 271_338, is_current: false },
];

// 浏览器预览里的占位图（appearance.ts 的 MOCK_IMAGE）右下角是暖浅灰，
// 给一对偏亮且跨度小的分位数，让预览走「墨色字裸压在图上」这条分支——
// 那是真机上最常见的一档，预览要能看出它长什么样。
const APPEARANCE: {
  background: string | null;
  tint: string | null;
  plate: { p10: number; p90: number } | null;
  veil: number;
  glass: GlassMode;
} = {
  background: "雪山黄昏.jpg",
  tint: "#4a6274",
  plate: { p10: 96, p90: 158 },
  veil: 0,
  // 与后端默认同档：浏览器预览默认落在保守的毛玻璃上，切过去才看得出液态到底加了什么。
  glass: "frost",
};

function appearanceDto() {
  return { ...APPEARANCE };
}

/** 与后端同口径的路径比较：Windows 不分大小写，且抹平斜杠与结尾分隔符。 */
function samePath(a: string, b: string): boolean {
  const norm = (p: string) => p.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return norm(a) === norm(b);
}

/** 当前目录在前，其余按记录顺序；mock 里除了一条故意设成不可达，其余都算存在。 */
function listGameDirs() {
  const current = {
    name: "当前文件夹",
    path: CONFIG.game_dir,
    is_current: true,
    available: true,
  };
  const others = EXTRA_DIRS.filter((d) => !samePath(d.path, CONFIG.game_dir)).map((d) => ({
    name: d.name,
    path: d.path,
    is_current: false,
    // D 盘那条当作没挂，好让界面上「不可达」这一态在浏览器里也能被看到。
    available: !d.path.startsWith("D:\\"),
  }));
  return [current, ...others];
}

// 版本级设置：mock 内存态，写进去能读回来，够阶段二的实例详情页离线开发。
const VERSION_SETTINGS = new Map<string, Record<string, unknown>>();

/** 复刻后端的隔离判定：本地数据强制 > 版本级覆盖 > 全局档位。 */
function resolveVersionSettings(versionId: string) {
  const saved = VERSION_SETTINGS.get(versionId) ?? {};
  const version = INSTALLED.versions.find((v) => v.id === versionId);
  const over = (saved.isolation as string) ?? "follow_global";
  const policy = CONFIG.isolation_policy;
  const byPolicy =
    policy === "all"
      ? true
      : policy === "disabled"
        ? false
        : policy === "mod_loaders_only"
          ? !!version?.has_mod_loader
          : policy === "non_release_only"
            ? !version?.is_release
            : !!version?.has_mod_loader || !version?.is_release;
  const isolated = over === "enabled" ? true : over === "disabled" ? false : byPolicy;
  return {
    description: (saved.description as string) ?? null,
    icon: (saved.icon as string) ?? null,
    favorite: (saved.favorite as boolean) ?? false,
    category: (saved.category as string) ?? null,
    isolation: over,
    working_dir: isolated ? `${CONFIG.game_dir}\\versions\\${versionId}` : CONFIG.game_dir,
    isolated,
    forced_by_local_data: false,
  };
}

// ---- Mod 生态 mock：版本表 / 落位矩阵 / 依赖计划 / 卷宗 / 更新 / 历史 / 崩溃诊断 ----

/** 版本表种子：platform / project_id / version_id 由调用方按入参回填，其余是共用的版本事实。 */
type VersionSeed = Omit<ModVersionInfo, "platform" | "project_id" | "version_id"> & {
  /** [Modrinth 版本 id, CurseForge fileId]。两边 id 形状差异很大，mock 也照实还原，
   *  否则前端很容易写出只在 Modrinth 下成立的 id 处理逻辑。 */
  ids: [string, number];
};

const FABRIC_API_ID = "P7dR8mSH";
const CLOTH_CONFIG_ID = "9s6osm5g";

// 以 Sodium 的真实发布节奏为骨架（Fabric 与 NeoForge 各出一份构建，1.20.1 之后不再出 Forge），
// 按发布时间倒序排列。三种通道、四条 MC 线、带 required/optional 依赖、以及「平台没给元数据」
// 的边界都齐了，够版本选择器与落位层吃到全部形状。
const MOD_VERSION_SEEDS: VersionSeed[] = [
  {
    ids: ["mL5rvsWq", 6482013],
    name: "Sodium 0.7.2 for MC 1.21.9",
    version_number: "mc1.21.9-0.7.2",
    release_channel: "release",
    game_versions: ["1.21.9", "1.21.8"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.7.2+mc1.21.9.jar",
    file_size: 1704213,
    sha1: "3f1c9a04b7d25e608cbb1a5f7d4e2c9018ab63f5",
    date_published: "2026-07-08T14:12:33Z",
    dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
  },
  {
    ids: ["Xq2pKdRn", 6482016],
    name: "Sodium 0.7.2 for MC 1.21.9 (NeoForge)",
    version_number: "mc1.21.9-0.7.2-neoforge",
    release_channel: "release",
    game_versions: ["1.21.9"],
    loaders: ["neoforge"],
    file_name: "sodium-neoforge-0.7.2+mc1.21.9.jar",
    file_size: 1798024,
    sha1: "c05b8e7719a3fd42d6b0e18c73a95f2ce4d1706b",
    date_published: "2026-07-08T14:09:02Z",
    dependencies: [],
  },
  {
    ids: ["7hVtQmLd", 6301884],
    name: "Sodium 0.7.0 Alpha for MC 1.21.5",
    version_number: "mc1.21.5-0.7.0-alpha",
    release_channel: "alpha",
    game_versions: ["1.21.5"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.7.0-alpha+mc1.21.5.jar",
    file_size: 1655090,
    sha1: "9d47ac2e0f6b83115c7ae920d3f8b6417e0c5da2",
    date_published: "2026-04-02T07:58:41Z",
    dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
  },
  {
    ids: ["JGdvfMPT", 6188402],
    name: "Sodium 0.6.13 for MC 1.21.4",
    version_number: "mc1.21.4-0.6.13",
    release_channel: "release",
    game_versions: ["1.21.4", "1.21.3"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.6.13+mc1.21.4.jar",
    file_size: 1612805,
    sha1: "5b2f0d81c6ea47399ff1c0d2b7e5a8341cf9e017",
    date_published: "2026-02-19T11:03:27Z",
    dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
  },
  {
    ids: ["Wn8cKtVb", 6188407],
    name: "Sodium 0.6.13 for MC 1.21.4 (NeoForge)",
    version_number: "mc1.21.4-0.6.13-neoforge",
    release_channel: "release",
    game_versions: ["1.21.4"],
    loaders: ["neoforge"],
    file_name: "sodium-neoforge-0.6.13+mc1.21.4.jar",
    file_size: 1701338,
    sha1: "a71e3c9d5480bb26f0e4172c98d5b3a06e2f4c88",
    date_published: "2026-02-19T10:58:55Z",
    dependencies: [],
  },
  {
    // 上传者只在 CurseForge 勾了 Client / Java 17，既没标 MC 版本也没标加载器。
    // 这类文件是 Unknown 档的来源，界面必须软提示放行而不是当成不兼容拦掉。
    ids: ["Rz4mYpQe", 6042771],
    name: "Sodium 0.6.9",
    version_number: "0.6.9",
    release_channel: "release",
    game_versions: [],
    loaders: [],
    file_name: "sodium-0.6.9.jar",
    file_size: null,
    sha1: null,
    date_published: "2026-01-14T20:30:12Z",
    dependencies: [],
  },
  {
    ids: ["Lp3sBnKw", 5904113],
    name: "Sodium 0.6.6 Beta for MC 1.21.1",
    version_number: "mc1.21.1-0.6.6-beta",
    release_channel: "beta",
    game_versions: ["1.21.1"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.6.6-beta+mc1.21.1.jar",
    file_size: 1588402,
    sha1: "e14b70a2c9df6538a0b1e47d92c3f8016ba5d47e",
    date_published: "2025-11-27T16:44:09Z",
    dependencies: [
      { project_id: FABRIC_API_ID, version_id: null, kind: "required" },
      { project_id: CLOTH_CONFIG_ID, version_id: null, kind: "optional" },
    ],
  },
  {
    ids: ["tPKvLNVW", 5772006],
    name: "Sodium 0.6.5 for MC 1.21.1",
    version_number: "mc1.21.1-0.6.5",
    release_channel: "release",
    game_versions: ["1.21.1", "1.21"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.6.5+mc1.21.1.jar",
    file_size: 1571244,
    sha1: "77c0f4a1b9e3d6250af8c31be74d095a2f6e18c3",
    date_published: "2025-09-16T09:21:50Z",
    dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
  },
  {
    ids: ["Yh6zAqDf", 5510934],
    name: "Sodium 0.5.11 for MC 1.20.1",
    version_number: "mc1.20.1-0.5.11",
    release_channel: "release",
    game_versions: ["1.20.1"],
    loaders: ["fabric"],
    file_name: "sodium-fabric-0.5.11+mc1.20.1.jar",
    file_size: 1284096,
    sha1: "2ab6108fd7c94e3350bb1f6d8a27ce401d93f5aa",
    date_published: "2025-06-04T13:07:18Z",
    dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
  },
];

/**
 * 把种子铺成某平台某工程的版本列表。CurseForge 的版本标识是 fileId 十进制字符串。
 *
 * 种子是照 Sodium 的真实版本表写的，结构价值在于覆盖了 Unknown 档（平台没给元数据）、beta 通道
 * 与必需/可选依赖三种情形。换工程时只把身份字段（名称、文件名）替换掉、保留这套结构，
 * 免得每个工程都手写一份版本表；否则点任何一个 Mod，弹层里显示的都是 Sodium，走查就失去意义。
 */
function modVersions(platform: PlatformId, projectId: string): ModVersionInfo[] {
  const hit = SEARCH_HITS.find((h) => h.project_id === projectId);
  const title = hit?.title ?? "Sodium";
  const slug = hit?.slug ?? "sodium";
  // 前置库自己不该依赖自己：点 Fabric API 时把种子里的 Fabric API 依赖去掉。
  const dropSelfDep = (deps: ModVersionInfo["dependencies"]) =>
    deps.filter((d) => d.project_id !== projectId);
  return MOD_VERSION_SEEDS.map(({ ids, ...rest }) => ({
    ...rest,
    platform,
    project_id: projectId,
    name: rest.name.replace("Sodium", title),
    file_name: rest.file_name.replace("sodium", slug),
    dependencies: dropSelfDep(rest.dependencies),
    version_id: platform === "curseforge" ? String(ids[1]) : ids[0],
  }));
}

/** 按 version_number 取种子铺出的版本；取不到直接抛，避免 mock 里悄悄写错版本号还一路装成功。 */
function versionByNumber(versions: ModVersionInfo[], versionNumber: string): ModVersionInfo {
  const found = versions.find((v) => v.version_number === versionNumber);
  if (!found) throw new Error(`[mock] 版本表里没有 version_number=${versionNumber}`);
  return found;
}

// 依赖项与更新目标用得到的两个真实工程版本（Fabric API 已装在实例里，Cloth Config 尚未装）。
const FABRIC_API_INSTALLED: ModVersionInfo = {
  version_id: "kJHFSCkq",
  project_id: FABRIC_API_ID,
  platform: "modrinth",
  name: "[1.21.1] Fabric API 0.115.0+1.21.1",
  version_number: "0.115.0+1.21.1",
  release_channel: "release",
  game_versions: ["1.21.1"],
  loaders: ["fabric"],
  file_name: "fabric-api-0.115.0+1.21.1.jar",
  file_size: 2244608,
  sha1: "b0d8e51947fca3260cd7f19e8a5b6142cd03f7e9",
  date_published: "2025-08-30T18:22:04Z",
  dependencies: [],
};

const FABRIC_API_LATEST: ModVersionInfo = {
  version_id: "P9pTU6Vs",
  project_id: FABRIC_API_ID,
  platform: "modrinth",
  name: "[1.21.1] Fabric API 0.119.2+1.21.1",
  version_number: "0.119.2+1.21.1",
  release_channel: "release",
  game_versions: ["1.21.1"],
  loaders: ["fabric"],
  file_name: "fabric-api-0.119.2+1.21.1.jar",
  file_size: 2310144,
  sha1: "1c73ea90b5f8d24671ae0c3948b21d5f60e7ab42",
  date_published: "2026-06-18T09:41:02Z",
  dependencies: [],
};

const CLOTH_CONFIG_LATEST: ModVersionInfo = {
  version_id: "sQ2fVbXm",
  project_id: CLOTH_CONFIG_ID,
  platform: "modrinth",
  name: "v15.0.140 - fabric - 1.21.1",
  version_number: "15.0.140+fabric",
  release_channel: "release",
  game_versions: ["1.21.1"],
  loaders: ["fabric"],
  file_name: "cloth-config-15.0.140-fabric.jar",
  file_size: 1092310,
  sha1: "4e9d1b7350ca82f6d0417be95c2a83f1607dbb2c",
  date_published: "2026-05-30T22:15:37Z",
  dependencies: [{ project_id: FABRIC_API_ID, version_id: null, kind: "required" }],
};

// 落位矩阵按三档手工编排（不是从 MOD_VERSION_SEEDS 机械推导的），目的是让落位层一次吃到
// 完美匹配 / 可能可行 / 不兼容三种视觉状态。顺序即后端约定的排序：同档内按实例 id 字典序。
function matchInstances(platform: PlatformId, projectId: string): InstanceMatch[] {
  const versions = modVersions(platform, projectId);
  const loaderMismatch = { kind: "mismatch", reason: "需要 Fabric 或 NeoForge，该实例装的是 Forge" } as const;
  return [
    {
      version_id: "1.21.1-Fabric",
      mc_version: "1.21.1",
      loaders: ["fabric"],
      compatibility: { kind: "match" },
      best_version: versionByNumber(versions, "mc1.21.1-0.6.5"),
      // 与 MODS / 卷宗种子对得上：这个实例里躺着 0.6.0，界面该把「安装」改成「更新」。
      already_installed: "sodium-fabric-0.6.0.jar",
    },
    {
      version_id: "1.21.4",
      mc_version: "1.21.4",
      loaders: [],
      compatibility: { kind: "unknown" },
      best_version: versionByNumber(versions, "0.6.9"),
      already_installed: null,
    },
    {
      version_id: "1.20.1-Forge_47.4.20",
      mc_version: "1.20.1",
      loaders: ["forge"],
      compatibility: loaderMismatch,
      best_version: null,
      already_installed: null,
    },
    {
      version_id: "World of Kivotos 2.0 beta",
      mc_version: "1.20.1",
      loaders: ["forge"],
      compatibility: loaderMismatch,
      best_version: null,
      already_installed: null,
    },
    {
      version_id: "测试服",
      mc_version: "1.20.1",
      loaders: ["forge"],
      compatibility: loaderMismatch,
      best_version: null,
      already_installed: null,
    },
  ];
}

function planInstall(platform: PlatformId, projectId: string, modVersionId: string): InstallPlan {
  const versions = modVersions(platform, projectId);
  const main = versions.find((v) => v.version_id === modVersionId);
  if (!main) {
    throw new Error(`[mock] 版本表里没有 version_id=${modVersionId}，请先用 listModVersions 取合法 id`);
  }
  // 主项自己就是这两个前置库时不能再把自己列成依赖，否则计划里会出现「因自己需要自己」。
  const deps = [
    // Fabric API 已经躺在 mods/ 里，本次只做依赖确认不重复下载。
    { version: FABRIC_API_INSTALLED, required_by: projectId, already_satisfied: true },
    { version: CLOTH_CONFIG_LATEST, required_by: projectId, already_satisfied: false },
  ].filter((d) => d.version.project_id !== projectId);

  return {
    items: [{ version: main, required_by: null, already_satisfied: false }, ...deps],
    skipped: [
      "Just Enough Items 是可选依赖（optional），未加入计划——只自动安装必需依赖。",
      "Indium 没有匹配 MC 1.21.1 与 Fabric 的版本，已跳过；缺它可能导致部分光影功能失效。",
    ],
  };
}

// 卷宗种子只记 Aurora 自己装的两个文件；jei 是手动丢进 mods 的，得靠哈希反查才补得上身份。
const LEDGER_SEED: LedgerEntry[] = [
  {
    file_name: "sodium-fabric-0.6.0.jar",
    platform: "modrinth",
    project_id: "AANobbMI",
    version_id: "bmMEkJhK",
    sha1: "8c41d0a7f36be92150ad7f2c48b1e6035da97f14",
    installed_at: 1785801600,
    installed_as_dependency_of: null,
  },
  {
    file_name: "fabric-api-0.115.0.jar",
    platform: "modrinth",
    project_id: FABRIC_API_ID,
    version_id: "kJHFSCkq",
    sha1: "b0d8e51947fca3260cd7f19e8a5b6142cd03f7e9",
    installed_at: 1785628800,
    installed_as_dependency_of: "AANobbMI",
  },
];

/** 哈希反查能认出来的存量 Mod：identify_installed_mods 调用后才会进卷宗。 */
const IDENTIFIABLE_SEED: LedgerEntry[] = [
  {
    file_name: "jei-19.21.0.jar",
    platform: "curseforge",
    project_id: "238222",
    version_id: "6104488",
    sha1: "f5a2c81d0eb3479a6c15d8f207be34c9d016ea77",
    installed_at: 1785628860,
    installed_as_dependency_of: null,
  },
];

// 历史种子（时间升序）：装 0.5.8 -> 升 0.5.11 -> 升 0.6.0 -> 升 Fabric API 又回滚。
// 最终磁盘状态刻意与 MODS 完全一致（sodium 0.6.0 / fabric-api 0.115.0 / jei 19.21.0），
// 免得历史页和 Mod 列表页互相打脸。
const HISTORY_SEED: HistoryEvent[] = [
  {
    kind: "install",
    id: "1785628800-1",
    at: 1785628800,
    files: ["sodium-fabric-0.5.8.jar", "fabric-api-0.115.0.jar"],
  },
  // 同一秒内的第二条：序号递增就是后端约定的 id 生成规则。
  { kind: "install", id: "1785628800-2", at: 1785628800, files: ["jei-19.21.0.jar"] },
  {
    kind: "update",
    id: "1785715200-1",
    at: 1785715200,
    file_name: "sodium-fabric-0.5.11.jar",
    old_file: "sodium-fabric-0.5.8.jar",
    from_version: "mc1.20.1-0.5.8",
    to_version: "mc1.20.1-0.5.11",
  },
  {
    kind: "update",
    id: "1785801600-1",
    at: 1785801600,
    file_name: "sodium-fabric-0.6.0.jar",
    old_file: "sodium-fabric-0.5.11.jar",
    from_version: "mc1.20.1-0.5.11",
    to_version: "mc1.21.1-0.6.0",
  },
  {
    kind: "update",
    id: "1785931200-1",
    at: 1785931200,
    file_name: "fabric-api-0.119.2.jar",
    old_file: "fabric-api-0.115.0.jar",
    from_version: "0.115.0+1.21.1",
    to_version: "0.119.2+1.21.1",
  },
  { kind: "rollback", id: "1785945600-1", at: 1785945600, reverted_event: "1785931200-1" },
];

/** 事件 id -> 该次更新残留的 `.old` 备份字节数。没进表的更新事件就是备份已不在磁盘上。 */
const BACKUP_SEED: [string, number][] = [["1785801600-1", 1284096]];

/** 每个实例一份可变状态：回滚要能被再次查到，哈希反查补身份只该生效一次。按实例分桶避免串味。 */
interface InstanceModState {
  events: HistoryEvent[];
  backups: Map<string, number>;
  ledger: LedgerEntry[];
}

const INSTANCE_MOD_STATE = new Map<string, InstanceModState>();

function modStateOf(versionId: string): InstanceModState {
  const cached = INSTANCE_MOD_STATE.get(versionId);
  if (cached) return cached;
  const fresh: InstanceModState = {
    events: [...HISTORY_SEED],
    backups: new Map(BACKUP_SEED),
    ledger: [...LEDGER_SEED],
  };
  INSTANCE_MOD_STATE.set(versionId, fresh);
  return fresh;
}

/** 返回不可回滚的中文原因；返回 null 表示可以回滚。检查与执行共用同一份判据，避免两处规则走偏。 */
function rollbackBlocker(state: InstanceModState, event: HistoryEvent): string | null {
  if (event.kind !== "update") return "只有更新事件可以回滚";
  const alreadyReverted = state.events.some(
    (e) => e.kind === "rollback" && e.reverted_event === event.id,
  );
  if (alreadyReverted) return "该次更新已经回滚过";
  if (!state.backups.has(event.id)) {
    return `备份文件 ${event.old_file}.old 已不在 mods 目录（可能已被清理）`;
  }
  return null;
}

function rollbackChecks(versionId: string): RollbackCheck[] {
  const state = modStateOf(versionId);
  return state.events.map((event) => {
    const blocker = rollbackBlocker(state, event);
    return { event_id: event.id, can_rollback: blocker === null, reason: blocker };
  });
}

function rollback(versionId: string, eventId: string): void {
  const state = modStateOf(versionId);
  const target = state.events.find((e) => e.id === eventId);
  if (!target) throw new Error(`[mock] 历史里没有事件 ${eventId}`);
  const blocker = rollbackBlocker(state, target);
  if (blocker) throw new Error(blocker);
  const at = Math.floor(Date.now() / 1000);
  const seq = state.events.filter((e) => e.at === at).length + 1;
  state.events.push({ kind: "rollback", id: `${at}-${seq}`, at, reverted_event: eventId });
  // 回滚把 .old 改回正式文件，备份随之消失——backup_size 也就跟着掉下去。
  state.backups.delete(eventId);
}

function checkUpdates(): UpdateCandidate[] {
  const versions = modVersions("modrinth", "AANobbMI");
  return [
    {
      file_name: "sodium-fabric-0.6.0.jar",
      current_version_id: "bmMEkJhK",
      latest: versionByNumber(versions, "mc1.21.1-0.6.5"),
    },
    {
      file_name: "fabric-api-0.115.0.jar",
      current_version_id: "kJHFSCkq",
      latest: FABRIC_API_LATEST,
    },
  ];
}

// 崩溃规则表：与 aurora-launch/src/crash.rs 的规则同源（取其中五条高频规则），同样按小写子串命中。
// mock 也做真判定，是为了让「日志里没有已知模式」这条空态分支在浏览器里也能被真实触发。
const CRASH_RULES: { patterns: string[]; diagnosis: Omit<CrashDiagnosis, "matched"> }[] = [
  {
    patterns: ["unsupportedclassversionerror", "class file version"],
    diagnosis: {
      category: "java_version_mismatch",
      summary: "Java 版本与游戏或 Mod 不匹配",
      advice:
        "改用目标版本要求的 Java：较新的 Minecraft 需要 Java 17 或 21，1.16 及更早需要 Java 8。可在设置中切换，或让 Aurora 自动下载匹配的运行时。",
      detail: "61.0",
    },
  },
  {
    patterns: ["outofmemoryerror", "java heap space", "could not reserve enough space for object heap"],
    diagnosis: {
      category: "out_of_memory",
      summary: "内存不足",
      advice:
        "调高最大内存（-Xmx），减少同时加载的 Mod 与高清材质；32 位 Java 无法分配大内存，请改用 64 位 Java。",
      detail: null,
    },
  },
  {
    patterns: ["incompatible mod set", "which is missing", "requires version"],
    diagnosis: {
      category: "missing_dependency",
      summary: "缺少前置 Mod 或依赖版本不满足",
      advice: "根据日志补齐缺失的前置 Mod（如 Fabric API），或把相关 Mod 调整到彼此兼容的版本。",
      detail: "sodium",
    },
  },
  {
    patterns: ["mixin apply failed", "error applying mixin"],
    diagnosis: {
      category: "mixin_failure",
      summary: "Mixin 注入失败（通常是 Mod 冲突或与游戏版本不兼容）",
      advice: "定位日志中报错的 Mod 并更新或移除；常见于 Mod 与当前 Minecraft 版本不匹配。",
      detail: "mixins.sodium.json:MixinLevelRenderer",
    },
  },
  {
    patterns: ["duplicatemodsfoundexception", "found a duplicate mod"],
    diagnosis: {
      category: "duplicate_mod",
      summary: "存在重复安装的 Mod",
      advice: "删除 mods 目录下重复的同一 Mod（只保留一个版本）。",
      detail: null,
    },
  },
];

function diagnose(logText: string): CrashDiagnosis[] {
  const lines = logText.split(/\r?\n/);
  const hits: CrashDiagnosis[] = [];
  for (const rule of CRASH_RULES) {
    const line = lines.find((l) => rule.patterns.some((p) => l.toLowerCase().includes(p)));
    if (line) hits.push({ ...rule.diagnosis, matched: line.trim().slice(0, 200) });
  }
  return hits;
}

// 最近一次崩溃的归档日志片段：故意同时命中「缺前置」与「Mixin 注入失败」两条规则。
const LAST_CRASH_LOG = [
  "[19:42:07] [main/INFO]: Loading Minecraft 1.21.1 with Fabric Loader 0.16.5",
  "[19:42:09] [main/INFO]: Loading 84 mods",
  "[19:42:11] [main/ERROR]: Incompatible mod set! Mod indium requires version 0.6.5 or later of sodium, which is missing!",
  "[19:42:11] [main/ERROR]: Mixin apply failed mixins.sodium.json:MixinLevelRenderer -> net.minecraft.class_761",
  "[19:42:12] [main/ERROR]: Game crashed during startup",
].join("\n");

function lastCrash(versionId: string): CrashReport {
  return {
    diagnoses: diagnose(LAST_CRASH_LOG),
    // mod id 与卷宗 join：sodium 对得上文件名，indium 没装过所以只报 id。
    suspects: [
      { mod_id: "sodium", file_name: "sodium-fabric-0.6.0.jar" },
      { mod_id: "indium", file_name: null },
    ],
    log_path: `${CONFIG.game_dir}\\versions\\${versionId}\\.aurora\\logs\\1785945600.log`,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function mockInvoke<T>(cmd: string, _args?: Record<string, unknown>): Promise<T> {
  await delay(180);

  // 带副作用或需读写内存态的命令必须在 table 之前短路：下面那张 table 的每个值都在构造时求值，
  // 把这类命令写进 table 会让任意一次 invoke 都触发一遍它们的副作用。
  if (cmd === "is_first_run") {
    return FIRST_RUN.pending as T;
  }
  if (cmd === "list_accounts") {
    return ACCOUNTS.slice() as T;
  }
  if (cmd === "current_account") {
    return (ACCOUNTS.find((a) => a.uuid === currentAccountUuid) ?? null) as T;
  }
  if (cmd === "create_offline_account") {
    return addOfflineAccount(String(_args?.name ?? "").trim()) as T;
  }
  if (cmd === "set_current_account") {
    setMockCurrentAccount(_args?.uuid as string);
    return undefined as T;
  }
  if (cmd === "remove_account") {
    removeMockAccount(_args?.uuid as string);
    return undefined as T;
  }
  if (cmd === "microsoft_login") {
    return (await microsoftLoginMock()) as T;
  }
  if (cmd === "authlib_login") {
    return loginMockAccount(AUTHLIB_SEED) as T;
  }
  if (cmd === "list_game_directories") {
    return listGameDirs() as T;
  }
  if (cmd === "discover_game_directories") {
    // 已经收下的不再重复推荐，与后端一致。
    return DISCOVERABLE.filter(
      (d) => !EXTRA_DIRS.some((e) => samePath(e.path, d.path)),
    ) as T;
  }
  if (cmd === "add_game_directory") {
    const path = _args?.path as string;
    const name = _args?.name as string;
    const hit = EXTRA_DIRS.find((d) => samePath(d.path, path));
    if (hit) hit.name = name;
    else EXTRA_DIRS.push({ name, path });
    return undefined as T;
  }
  if (cmd === "remove_game_directory") {
    const path = _args?.path as string;
    const before = EXTRA_DIRS.length;
    const kept = EXTRA_DIRS.filter((d) => !samePath(d.path, path));
    EXTRA_DIRS.length = 0;
    EXTRA_DIRS.push(...kept);
    return (EXTRA_DIRS.length !== before) as T;
  }
  if (cmd === "switch_game_directory") {
    const path = _args?.path as string;
    if (!samePath(path, CONFIG.game_dir)) {
      const previous = CONFIG.game_dir;
      const kept = EXTRA_DIRS.filter((d) => !samePath(d.path, path));
      EXTRA_DIRS.length = 0;
      EXTRA_DIRS.push(...kept, { name: "上一个目录", path: previous });
      CONFIG.game_dir = path;
    }
    return undefined as T;
  }
  if (cmd === "complete_first_run") {
    CONFIG.game_dir = _args?.gameDir as string;
    for (const extra of (_args?.extras as { name: string; path: string }[]) ?? []) {
      if (!EXTRA_DIRS.some((d) => samePath(d.path, extra.path))) EXTRA_DIRS.push(extra);
    }
    FIRST_RUN.pending = false;
    return undefined as T;
  }
  // 外观：浏览器里也要能试出「有背景/没背景」两种版式，所以这几条同样维护内存态。
  if (cmd === "get_appearance") {
    return appearanceDto() as T;
  }
  if (cmd === "list_backgrounds") {
    return BACKGROUNDS.map((b) => ({ ...b, is_current: b.file === APPEARANCE.background })) as T;
  }
  if (cmd === "import_background") {
    // 真实实现会把图复制进图库并转码；mock 只按路径末段编一个条目。
    const path = (_args?.path as string) ?? "新背景.png";
    const stem = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "新背景";
    const file = `${stem}.jpg`;
    if (!BACKGROUNDS.some((b) => b.file === file)) {
      BACKGROUNDS.push({ file, width: 1920, height: 1080, bytes: 412_336, is_current: false });
    }
    APPEARANCE.background = file;
    APPEARANCE.tint = "#4a6274";
    APPEARANCE.plate = { p10: 96, p90: 158 };
    return appearanceDto() as T;
  }
  if (cmd === "set_background") {
    const file = (_args?.file as string | null) ?? null;
    APPEARANCE.background = file;
    APPEARANCE.tint = file ? "#4a6274" : null;
    APPEARANCE.plate = file ? { p10: 96, p90: 158 } : null;
    return appearanceDto() as T;
  }
  if (cmd === "remove_background") {
    const file = _args?.file as string;
    const kept = BACKGROUNDS.filter((b) => b.file !== file);
    BACKGROUNDS.length = 0;
    BACKGROUNDS.push(...kept);
    if (APPEARANCE.background === file) {
      APPEARANCE.background = null;
      APPEARANCE.tint = null;
      APPEARANCE.plate = null;
    }
    return appearanceDto() as T;
  }
  if (cmd === "set_glass_mode") {
    APPEARANCE.glass = _args?.glass as GlassMode;
    return appearanceDto() as T;
  }
  if (cmd === "set_background_veil") {
    // 与后端同样钳到上限，否则浏览器里能拖出后端根本不接受的值。
    APPEARANCE.veil = Math.min(Math.max(_args?.veil as number, 0), 60);
    return appearanceDto() as T;
  }
  if (cmd === "get_version_settings") {
    return resolveVersionSettings(_args?.versionId as string) as T;
  }
  if (cmd === "set_version_settings") {
    const id = _args?.versionId as string;
    VERSION_SETTINGS.set(id, (_args?.settings as Record<string, unknown>) ?? {});
    return resolveVersionSettings(id) as T;
  }
  if (cmd === "managed_modpack_status") {
    return ((_args?.versionId as string) === MOCK_MANAGED_VERSION_ID
      ? MOCK_MANAGED_STATUS
      : null) as T;
  }
  if (cmd === "managed_modpack_files") {
    return ((_args?.versionId as string) === MOCK_MANAGED_VERSION_ID
      ? MOCK_MANAGED_FILES
      : null) as T;
  }
  if (cmd === "sync_managed_modpack") {
    throw browserPreviewWriteError((_args?.targetVersion as string) || "latest");
  }
  if (cmd === "install_managed_modpack") {
    throw browserPreviewWriteError("latest");
  }

  // Mod 生态这批命令要么读入参、要么改内存态，全部走短路分支，一条都不能进 table。
  if (cmd === "list_mod_versions") {
    const all = modVersions(_args?.platform as PlatformId, _args?.projectId as string);
    // 过滤口径与后端一致：两个条件都是「传空数组即不过滤」，非空则要求有交集。
    // mock 不实现过滤的话，落位层的「只看配得上的」在浏览器里看不出任何差别。
    const wantVersions = (_args?.gameVersions as string[]) ?? [];
    const wantLoaders = (_args?.loaders as string[]) ?? [];
    return all.filter((v) => {
      const okVersion =
        wantVersions.length === 0 || v.game_versions.some((g) => wantVersions.includes(g));
      const okLoader = wantLoaders.length === 0 || v.loaders.some((l) => wantLoaders.includes(l));
      return okVersion && okLoader;
    }) as T;
  }
  if (cmd === "match_instances") {
    return matchInstances(_args?.platform as PlatformId, _args?.projectId as string) as T;
  }
  if (cmd === "plan_install") {
    return planInstall(
      _args?.platform as PlatformId,
      _args?.projectId as string,
      _args?.modVersionId as string,
    ) as T;
  }
  if (cmd === "identify_installed_mods") {
    const state = modStateOf(_args?.versionId as string);
    const known = new Set(state.ledger.map((e) => e.file_name));
    const found = IDENTIFIABLE_SEED.filter((e) => !known.has(e.file_name));
    state.ledger.push(...found);
    // 第二次调用返回 0：能反查的都补完了，剩下的是本地 Mod，本就没有来源可查。
    return found.length as T;
  }
  if (cmd === "check_updates") {
    if ((_args?.versionId as string) === MOCK_MANAGED_VERSION_ID) return [] as T;
    return checkUpdates() as T;
  }
  if (cmd === "list_history") {
    const history: History = { events: modStateOf(_args?.versionId as string).events };
    return history as T;
  }
  if (cmd === "rollback_checks") {
    return rollbackChecks(_args?.versionId as string) as T;
  }
  if (cmd === "rollback") {
    rollback(_args?.versionId as string, _args?.eventId as string);
    return undefined as T;
  }
  if (cmd === "backup_size") {
    const state = modStateOf(_args?.versionId as string);
    let total = 0;
    for (const bytes of state.backups.values()) total += bytes;
    return total as T;
  }
  if (cmd === "diagnose_crash") {
    // 临时粘贴的日志没有归档文件，log_path 就该是 null，别造一个打不开的路径出来。
    const report: CrashReport = {
      diagnoses: diagnose(_args?.logText as string),
      suspects: [],
      log_path: null,
    };
    return report as T;
  }
  if (cmd === "last_crash") {
    return lastCrash(_args?.versionId as string) as T;
  }
  if (cmd === "list_ledger") {
    const ledger: Ledger = { entries: modStateOf(_args?.versionId as string).ledger };
    return ledger as T;
  }

  const table: Record<string, unknown> = {
    get_config: CONFIG,
    list_installed: INSTALLED,
    install_version: { vanilla: { id: "1.21.1", libraries: 42, assets: 3200, natives: 6 }, loader: null },
    launch_game: { pid: 73136 },
    stop_game: undefined,
    detect_java: [
      { path: "C:\\Program Files\\Java\\jdk-21\\bin\\java.exe", version: { major: 21, minor: 0, security: 2, build: 13, raw: "21.0.2" }, is_64bit: true, vendor: "Eclipse Temurin", source: "registry" },
      { path: "C:\\Program Files\\Java\\jdk-17\\bin\\java.exe", version: { major: 17, minor: 0, security: 10, build: 7, raw: "17.0.10" }, is_64bit: true, vendor: "Microsoft", source: "registry" },
    ],
    install_java: { component: "java-runtime-gamma", version: { major: 21, minor: 0, security: 2, build: 13, raw: "21.0.2" }, java_executable: "" },
    update_config: undefined,
    set_game_directory: undefined,
    search_resources: searchResult(
      (_args?.query as string) || "",
      (_args?.resourceType as string) || "mod",
      (_args?.sort as string) || "relevance",
    ),
    install_mod: { file_name: "sodium.jar", path: "", platform: "modrinth" },
    list_mods: MODS,
    set_mod_enabled: "sodium-fabric-0.6.0.jar",
  };
  if (!(cmd in table)) throw new Error(`[mock] 未实现命令: ${cmd}`);
  return table[cmd] as T;
}

// 事件订阅 mock：launch/install 时发几帧假进度，好测启动动画与进度条。
export async function mockListen<T>(
  event: string,
  handler: (e: { event: string; payload: T }) => void,
): Promise<UnlistenFn> {
  if (event === "aurora://core-event") {
    const stages = ["解析 Java", "合并版本清单", "补全资源文件", "拼装启动命令"];
    stages.forEach((message, i) =>
      setTimeout(() => handler({ event, payload: { kind: "stage", message } as T }), 400 * (i + 1)),
    );
  }
  if (event === "aurora://device-code") {
    // 这一条与别的事件不同：它由 microsoft_login 那次调用现场推送，故必须真把订阅者记下来，
    // 返回的 unlisten 也要真把它摘掉——账户页每次登录都会重新订一次，不摘就越积越多。
    const notify = (code: DeviceCode) => handler({ event, payload: code as T });
    deviceCodeHandlers.add(notify);
    return () => {
      deviceCodeHandlers.delete(notify);
    };
  }
  return () => {};
}
