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

function manifest() {
  const versions: unknown[] = [];
  const push = (id: string, t: string) =>
    versions.push({ id, release_type: t, url: "", time: "2026-01-01", release_time: "2026-01-01", sha1: null, compliance_level: 1 });
  ["1.21.4", "1.21.3", "1.21.1", "1.21", "1.20.6", "1.20.4", "1.20.1", "1.19.4", "1.18.2", "1.16.5", "1.12.2", "1.7.10"].forEach((v) => push(v, "release"));
  ["25w05a", "24w45a", "24w40a", "23w13a_or_b"].forEach((v) => push(v, "snapshot"));
  return { latest: { release: "1.21.4", snapshot: "25w05a" }, versions };
}

const SEARCH_HITS = [
  { title: "Sodium", author: "jellysquid3", downloads: 45_200_000, categories: ["optimization"], resource_type: "mod", desc: "现代化 OpenGL 渲染引擎，大幅提升帧率与流畅度。" },
  { title: "Iris Shaders", author: "coderbot", downloads: 22_800_000, categories: ["optimization", "utility"], resource_type: "mod", desc: "在 Sodium 之上加载 OptiFine/光影包。" },
  { title: "Fabric API", author: "modmuss50", downloads: 89_400_000, categories: ["library"], resource_type: "mod", desc: "Fabric 生态的核心互操作 API。" },
  { title: "JEI", author: "mezz", downloads: 67_100_000, categories: ["utility"], resource_type: "mod", desc: "物品与配方查看，Just Enough Items。" },
  { title: "Create", author: "simibubi", downloads: 31_500_000, categories: ["technology"], resource_type: "mod", desc: "机械动力：齿轮、传动与自动化建造。" },
  { title: "Complementary Shaders", author: "EminGT", downloads: 12_300_000, categories: ["shader"], resource_type: "shader", desc: "高品质光影，兼顾观感与性能。" },
];

function searchResult(query: string, type: string) {
  const q = (query || "").toLowerCase();
  const hits = SEARCH_HITS.filter((h) => (type === "mod" ? true : h.resource_type === type))
    .filter((h) => !q || h.title.toLowerCase().includes(q))
    .map((h, i) => ({
      platform: i % 2 === 0 ? "modrinth" : "curseforge",
      project_id: "proj-" + i,
      slug: h.title.toLowerCase().replace(/\s+/g, "-"),
      title: h.title,
      description: h.desc,
      author: h.author,
      downloads: h.downloads,
      follows: Math.round(h.downloads / 300),
      icon_url: null,
      categories: h.categories,
      resource_type: h.resource_type,
      date_modified: "2026-06-01",
      page_url: null,
    }));
  return { hits, errors: [] };
}

const MODS = [
  { path: "", file_name: "sodium-fabric-0.6.0.jar", enabled: true, metadata: { mod_id: "sodium", name: "Sodium", version: "0.6.0", description: null, authors: ["jellysquid3"], loader: "fabric", format: "fabric.mod.json" } },
  { path: "", file_name: "fabric-api-0.115.0.jar", enabled: true, metadata: { mod_id: "fabric-api", name: "Fabric API", version: "0.115.0", description: null, authors: [], loader: "fabric", format: "fabric.mod.json" } },
  { path: "", file_name: "jei-19.21.0.jar", enabled: false, metadata: { mod_id: "jei", name: "Just Enough Items", version: "19.21.0", description: null, authors: ["mezz"], loader: "fabric", format: "fabric.mod.json" } },
];

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function mockInvoke<T>(cmd: string, _args?: Record<string, unknown>): Promise<T> {
  await delay(180);
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
    search_resources: searchResult((_args?.query as string) || "", (_args?.resourceType as string) || "mod"),
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
