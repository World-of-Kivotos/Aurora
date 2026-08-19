import { validateModpackPointerUrl } from "./modpack-ui";

/**
 * 「带着这条整合包地址去装一次」的深链。
 *
 * 目的地原来是下载页的整合包 tab。Aurora 收敛成 World of Kivotos 专用启动器之后那个 tab 撤了，
 * 装游戏的唯一入口是启动屏，深链跟着改指 `/`——否则卷宗页在同步冲突时给出的
 * 「安装新版本」会跳进一个不存在的 tab，被路由兜底弹回启动屏，点了等于没点。
 */
export function managedModpackInstallRoute(pointerUrl: string): string {
  const searchParams = new URLSearchParams();
  searchParams.set("pointer", pointerUrl);
  return `/?${searchParams.toString()}`;
}

export interface ManagedModpackInstallIntent {
  /** 是不是从深链进来的。启动屏据此立刻把安装面板摆出来，不等实例扫描出结果。 */
  requested: boolean;
  /** 预填地址；参数不可信时为 null，由调用方回落到内置地址。 */
  pointerUrl: string | null;
}

/**
 * 启动屏解读地址栏：要不要直接展开安装面板，以及地址预填什么。
 *
 * 两个信号刻意分开取。意图由参数「在不在」表达，而不是它「合不合法」：
 * 带着一条脏地址过来的人一样是来装游戏的，此时该做的是摆出面板并把地址退回内置值，
 * 而不是当作他没点过那个按钮——后者会让玩家盯着一个什么都没发生的启动屏。
 */
export function managedModpackInstallIntent(
  searchParams: URLSearchParams,
): ManagedModpackInstallIntent {
  return {
    requested: searchParams.has("pointer"),
    pointerUrl: managedModpackPointerFromSearch(searchParams),
  };
}

export function managedModpackPointerFromSearch(
  searchParams: URLSearchParams,
): string | null {
  const pointerUrl = searchParams.get("pointer");
  if (pointerUrl === null) return null;

  const trimmed = pointerUrl.trim();
  return validateModpackPointerUrl(trimmed) === null ? trimmed : null;
}
