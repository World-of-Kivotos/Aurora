// 前端预取与 TTL 缓存层。
//
// 背景：后端 list_manifest / search_resources 都是直连上游（Mojang 清单、Modrinth/CurseForge 搜索），
// 没有任何缓存——每次进下载页都要等一次网络往返，五个 tab 就是五次冷启动。
// 这里在启动器起来后的空闲期把下载页各 tab 的首屏并发预取一遍，进页面直接命中内存，零骨架闪烁。
//
// 三件事：TTL 缓存、并发去重（同 key 的第二个调用复用在途 promise）、启动编排。

import {
  listManifest,
  searchResources,
  type ManifestDto,
  type ModLoader,
  type ResourceType,
  type SearchResultDto,
  type SortField,
} from "./ipc";

interface Entry {
  value: unknown;
  at: number;
}

const store = new Map<string, Entry>();
const inflight = new Map<string, Promise<unknown>>();

/** 版本清单变动慢（Mojang 发版级别），搜索结果放宽到分钟级即可覆盖一次使用会话。 */
export const TTL = {
  manifest: 10 * 60_000,
  search: 5 * 60_000,
} as const;

export const MANIFEST_KEY = "manifest";

/** 搜索缓存键：参数一字不差才算同一份结果，避免筛选变了还吃旧缓存。 */
export function searchKey(
  type: ResourceType,
  sort: SortField,
  query: string,
  loaders: ModLoader[],
  gameVersions: string[],
): string {
  return ["search", type, sort, query.trim().toLowerCase(), loaders.join("+"), gameVersions.join("+")].join(":");
}

/** 同步读缓存：命中且未过期才返回，否则 undefined。页面用它决定首帧是直接出内容还是出骨架。 */
export function peek<T>(key: string, ttl: number): T | undefined {
  const hit = store.get(key);
  if (!hit) return undefined;
  if (Date.now() - hit.at > ttl) {
    store.delete(key);
    return undefined;
  }
  return hit.value as T;
}

/** 取数：命中缓存直接返回；有在途请求复用；否则真正发起。loader 抛错原样冒泡，且不写缓存、不留在途。 */
export function cached<T>(key: string, ttl: number, loader: () => Promise<T>): Promise<T> {
  const hit = peek<T>(key, ttl);
  if (hit !== undefined) return Promise.resolve(hit);

  const running = inflight.get(key);
  if (running) return running as Promise<T>;

  const p = loader()
    .then((value) => {
      store.set(key, { value, at: Date.now() });
      return value;
    })
    .finally(() => {
      inflight.delete(key);
    });

  inflight.set(key, p);
  return p;
}

/** 手动作废：安装完新版本后清掉清单缓存，下次进页面重拉。 */
export function invalidate(key: string): void {
  store.delete(key);
}

export const fetchManifest = (): Promise<ManifestDto> => cached(MANIFEST_KEY, TTL.manifest, listManifest);

export function fetchSearch(
  type: ResourceType,
  sort: SortField,
  query: string,
  loaders: ModLoader[],
  gameVersions: string[],
): Promise<SearchResultDto> {
  const key = searchKey(type, sort, query, loaders, gameVersions);
  return cached(key, TTL.search, () =>
    searchResources({ query, resourceType: type, loaders, gameVersions, sort, limit: 30, offset: 0 }),
  );
}

/** 下载页首屏参数：预取与页面初始状态必须一致，否则预取白做（键对不上）。 */
export const DEFAULT_SORT: SortField = "relevance";
export const PREFETCH_TYPES: ResourceType[] = ["mod", "modpack", "resource_pack", "shader"];

export const defaultSearchKey = (type: ResourceType) => searchKey(type, DEFAULT_SORT, "", [], []);

/**
 * 启动预取：清单 + 四类资源首屏并发拉取。
 * 单项失败只记录不抛——预取是纯优化路径，真正进页面时会再取一次并把错误正常呈现给用户；
 * 若这里 throw，会变成没人接的 unhandled rejection，反而掩盖问题。失败项不写缓存，下次自动重试。
 */
export function prefetchDownloadTabs(): void {
  const tasks: Array<[string, Promise<unknown>]> = [
    [MANIFEST_KEY, fetchManifest()],
    ...PREFETCH_TYPES.map(
      (t) => [defaultSearchKey(t), fetchSearch(t, DEFAULT_SORT, "", [], [])] as [string, Promise<unknown>],
    ),
  ];
  for (const [key, p] of tasks) {
    p.catch((e) => console.warn(`[prefetch] ${key} 预取失败，进入页面时会重试：`, e));
  }
}

/** 在浏览器空闲时机启动预取，避免和首屏渲染抢主线程。无 requestIdleCallback 的环境退到定时器。 */
export function schedulePrefetch(): void {
  const run = () => prefetchDownloadTabs();
  const ric = (window as unknown as { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => void })
    .requestIdleCallback;
  if (ric) ric(run, { timeout: 2000 });
  else window.setTimeout(run, 400);
}
