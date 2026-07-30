// 浏览器 mock：不在 Tauri 环境时（如 `pnpm dev` 用 puppeteer/浏览器看 UI），用假数据驱动全部页面，
// 让前端能脱离 Rust 后端独立开发/截图。仅开发期生效——正式打包在 Tauri 里走真 IPC（见 tauri-bridge.ts）。

import type { UnlistenFn } from "@tauri-apps/api/event";

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

const ACCOUNTS = [
  { uuid: "853c80ef3c3749fdaa49938b674adae6", name: "Shinoyuki_Miyako", account_type: "microsoft" },
  { uuid: "069a79f444e94726a5befca90e38aaf5", name: "Steve", account_type: "offline" },
];

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

// 版本清单：Mojang version_manifest_v2 的真实子集（id / release_type / release_time 均为线上原值），
// 四种 release_type 都覆盖到，好让类型筛选与日期排版在开发期就吃到真实形状。
const MANIFEST_ROWS: [string, string, string][] = [
  ["26.2", "release", "2026-06-16"],
  ["26.1.2", "release", "2026-04-09"],
  ["26.1.1", "release", "2026-04-01"],
  ["26.1", "release", "2026-03-24"],
  ["1.21.11", "release", "2025-12-09"],
  ["1.21.10", "release", "2025-10-07"],
  ["1.21.9", "release", "2025-09-30"],
  ["1.21.8", "release", "2025-07-17"],
  ["1.21.7", "release", "2025-06-30"],
  ["1.21.6", "release", "2025-06-17"],
  ["1.21.5", "release", "2025-03-25"],
  ["1.21.4", "release", "2024-12-03"],
  ["1.21.3", "release", "2024-10-23"],
  ["1.21.2", "release", "2024-10-22"],
  ["1.21.1", "release", "2024-08-08"],
  ["1.21", "release", "2024-06-13"],
  ["1.20.6", "release", "2024-04-29"],
  ["1.20.5", "release", "2024-04-23"],
  ["1.20.4", "release", "2023-12-07"],
  ["1.20.3", "release", "2023-12-04"],
  ["1.20.2", "release", "2023-09-20"],
  ["1.20.1", "release", "2023-06-12"],
  ["1.20", "release", "2023-06-02"],
  ["1.19.4", "release", "2023-03-14"],
  ["1.19.3", "release", "2022-12-07"],
  ["1.19.2", "release", "2022-08-05"],
  ["26.3-snapshot-6", "snapshot", "2026-07-28"],
  ["26.3-snapshot-5", "snapshot", "2026-07-21"],
  ["26.3-snapshot-4", "snapshot", "2026-07-16"],
  ["26.3-snapshot-3", "snapshot", "2026-07-07"],
  ["26.3-snapshot-2", "snapshot", "2026-06-30"],
  ["26.3-snapshot-1", "snapshot", "2026-06-23"],
  ["26.2-rc-2", "snapshot", "2026-06-12"],
  ["26.2-rc-1", "snapshot", "2026-06-11"],
  ["26.2-pre-6", "snapshot", "2026-06-10"],
  ["26.2-pre-5", "snapshot", "2026-06-08"],
  ["26.2-pre-4", "snapshot", "2026-06-04"],
  ["26.2-pre-3", "snapshot", "2026-06-02"],
  ["26.2-pre-2", "snapshot", "2026-05-28"],
  ["26.2-pre-1", "snapshot", "2026-05-26"],
  ["b1.8.1", "old_beta", "2011-09-18"],
  ["b1.8", "old_beta", "2011-09-14"],
  ["b1.7.3", "old_beta", "2011-07-07"],
  ["b1.7.2", "old_beta", "2011-06-30"],
  ["b1.7", "old_beta", "2011-06-29"],
  ["b1.6.6", "old_beta", "2011-05-30"],
  ["a1.2.6", "old_alpha", "2010-12-02"],
  ["a1.2.5", "old_alpha", "2010-11-30"],
  ["a1.2.4_01", "old_alpha", "2010-11-29"],
  ["a1.2.3_04", "old_alpha", "2010-11-25"],
];

function manifest() {
  const versions = MANIFEST_ROWS.map(([id, release_type, day]) => ({
    id,
    release_type,
    url: `https://piston-meta.mojang.com/v1/packages/mock/${id}.json`,
    time: day,
    release_time: day,
    sha1: null,
    compliance_level: 1,
  }));
  return { latest: { release: "26.2", snapshot: "26.3-snapshot-6" }, versions };
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

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function mockInvoke<T>(cmd: string, _args?: Record<string, unknown>): Promise<T> {
  await delay(180);

  // 带副作用或需读写内存态的命令必须在 table 之前短路：下面那张 table 的每个值都在构造时求值，
  // 把这类命令写进 table 会让任意一次 invoke 都触发一遍它们的副作用。
  if (cmd === "get_version_settings") {
    return resolveVersionSettings(_args?.versionId as string) as T;
  }
  if (cmd === "set_version_settings") {
    const id = _args?.versionId as string;
    VERSION_SETTINGS.set(id, (_args?.settings as Record<string, unknown>) ?? {});
    return resolveVersionSettings(id) as T;
  }

  const table: Record<string, unknown> = {
    get_config: CONFIG,
    list_installed: INSTALLED,
    current_account: ACCOUNTS[0],
    list_accounts: ACCOUNTS,
    create_offline_account: ACCOUNTS[1],
    microsoft_login: ACCOUNTS[0],
    authlib_login: ACCOUNTS[0],
    set_current_account: undefined,
    remove_account: undefined,
    list_manifest: manifest(),
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
  return () => {};
}
