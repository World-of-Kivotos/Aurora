import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  ManagedModpackPanel,
  ModpackFileManagement,
  ModpackFileOwnership,
} from "./ManagedModpackPanel";
import { Button } from "./Button";
import { ModpackInstallFlow } from "./ModpackInstallFlow";
import {
  managedModpackInstallIntent,
  managedModpackInstallRoute,
} from "../lib/modpack-navigation";
import type { ManagedModpackStatus } from "../lib/modpack-ui";

const READY: ManagedModpackStatus = {
  kind: "ready",
  subscription: {
    pack_id: "wok",
    pointer_url: "https://api.mcwok.cn/api/v1/pack/latest",
  },
  versions: {
    installed_version: "1.9.0",
    latest: {
      pack_id: "wok",
      version: "2.0.0",
      manifest_url: "https://api.mcwok.cn/api/v1/pack/manifest/2.0.0",
      released_at: "2026-08-17T12:00:00Z",
      note: "新周目：矿洞维度重做",
      min_launcher_version: "0.3.0",
    },
  },
  source: "network",
  checked_at: "1786968300",
};

interface ClickableProps {
  children?: ReactNode;
  onClick?: () => void;
}

function findButton(node: ReactNode, label: string): ReactElement<ClickableProps> | null {
  if (!isValidElement<ClickableProps>(node)) return null;
  if (node.type === Button && node.props.children === label) return node;
  for (const child of Children.toArray(node.props.children)) {
    const match = findButton(child, label);
    if (match) return match;
  }
  return null;
}

describe("ManagedModpackPanel", () => {
  it("shows installed and available versions with an update action", () => {
    const html = renderToStaticMarkup(
      <ManagedModpackPanel
        status={READY}
        instanceId="wok-1.20.1-forge"
        sync={{ kind: "idle" }}
        onCheck={() => undefined}
        onSync={() => undefined}
        onInstallNewVersion={() => undefined}
      />,
    );

    expect(html).toContain("当前版本");
    expect(html).toContain("1.9.0");
    expect(html).toContain("可用版本");
    expect(html).toContain("2.0.0");
    expect(html).toContain("更新整合包");
    expect(html).toContain("单独的平台更新检查已停用");
  });

  it("keeps the failing file and disk guidance visible", () => {
    const html = renderToStaticMarkup(
      <ManagedModpackPanel
        status={READY}
        instanceId="wok-1.20.1-forge"
        sync={{
          kind: "failed",
          target_version: "2.0.0",
          stage: "downloading_files",
          failure: {
            kind: "disk_full",
            file_path: "mods/wok-core-2.0.0.jar",
            required_bytes: 16 * 1024 * 1024,
            available_bytes: 4 * 1024 * 1024,
          },
        }}
        onCheck={() => undefined}
        onSync={() => undefined}
        onInstallNewVersion={() => undefined}
      />,
    );

    expect(html).toContain("磁盘空间不足：mods/wok-core-2.0.0.jar");
    expect(html).toContain("本次写入需要 16.0 MB，当前可用 4.0 MB");
    expect(html).toContain("重试同步");
  });

  // 冲突这一支的措辞必须被断言钉住：单实例收敛之后，这颗按钮装出来的实例会顶掉
  // config.selected_version，旧实例目录留在磁盘上却再没有任何界面进得去。
  // 一旦文案退回「作为新实例安装」这类并存承诺，玩家会以为随时切得回旧版本，
  // 而这种误解在界面上是看不出来的——只能靠断言守住。
  it("opens one-click installation after a conflict without promising the old instance stays reachable", () => {
    const onCheck = vi.fn();
    const onSync = vi.fn();
    const onInstallNewVersion = vi.fn();
    const panel = ManagedModpackPanel({
      status: READY,
      instanceId: "wok-1.20.1-forge",
      sync: {
        kind: "failed",
        target_version: "2.0.0",
        stage: "resolving_manifest",
        failure: { kind: "conflict", detail: "当前 Minecraft 版本无法原地切换" },
      },
      onCheck,
      onSync,
      onInstallNewVersion,
    });
    const html = renderToStaticMarkup(panel);

    expect(html).toContain("整合包状态已变化");
    expect(html).toContain("安装新版本");
    expect(html).not.toContain("作为新实例安装");
    expect(html).toContain("wok-1.20.1-forge");
    expect(html).toContain("不再出现在启动器中");
    expect(html).not.toContain("重新检查");
    expect(html).not.toContain("重试同步");

    const installButton = findButton(panel, "安装新版本");
    expect(installButton).not.toBeNull();
    installButton?.props.onClick?.();
    expect(onInstallNewVersion).toHaveBeenCalledOnce();
    expect(onCheck).not.toHaveBeenCalled();
    expect(onSync).not.toHaveBeenCalled();
  });

  it("renders file ownership without disguising managed files as player files", () => {
    const managed = renderToStaticMarkup(<ModpackFileOwnership owner="modpack" />);
    const player = renderToStaticMarkup(<ModpackFileOwnership owner="player" />);
    expect(managed).toContain("整合包提供");
    expect(managed).toContain("不能单独移除");
    expect(player).toContain("玩家自装");
  });

  it("never renders a remove action for a managed file", () => {
    const managed = renderToStaticMarkup(<ModpackFileManagement owner="modpack" />);
    const player = renderToStaticMarkup(
      <ModpackFileManagement owner="player" removing={false} onRemove={() => undefined} />,
    );
    expect(managed).not.toContain("<button");
    expect(player).toContain("<button");
    expect(player).toContain("移除");
  });
});

const BUILT_IN = {
  label: "WOK 地址",
  pointer_url: "https://api.mcwok.cn/api/v1/pack/latest",
};

/**
 * 深链落点的真实装配：启动屏读地址栏走 managedModpackInstallIntent，
 * 读出来的两个信号（要不要展开、预填什么）原样喂给这块面板，这里把那两步接起来跑。
 *
 * 不直接渲染 <Home />：它挂在 Toast 与 Appearance 两个 Provider 上，
 * 而 ToastProvider 会 createPortal 到 document.body，renderToStaticMarkup 渲不出来。
 */
function renderRoutedInstaller(route: string) {
  const parsedRoute = new URL(route, "https://aurora.local");
  const intent = managedModpackInstallIntent(parsedRoute.searchParams);
  return {
    parsedRoute,
    intent,
    html: renderToStaticMarkup(
      <ModpackInstallFlow
        builtIn={BUILT_IN}
        initialPointerUrl={intent.pointerUrl ?? undefined}
        state={{ kind: "idle" }}
        onInstall={() => undefined}
      />,
    ),
  };
}

describe("ModpackInstallFlow", () => {
  it("offers the built-in pointer and the full one-click install sequence", () => {
    const html = renderToStaticMarkup(
      <ModpackInstallFlow
        builtIn={BUILT_IN}
        state={{ kind: "idle" }}
        onInstall={() => undefined}
      />,
    );

    expect(html).toContain('value="https://api.mcwok.cn/api/v1/pack/latest"');
    expect(html).toContain("读取整合包");
    expect(html).toContain("安装 Minecraft");
    expect(html).toContain("安装加载器");
    expect(html).toContain("同步整合包");
    // 面板不得自带安装触发器: 启动屏右下角那颗主操作键在未安装时本身就是 Download,
    // 两处并列会让新玩家对着两个「下载」发愣。这条断言守的就是它别再被加回来。
    expect(html).not.toContain("安装游戏");
    expect(html).toContain("点右下角的 Download 开始");
  });

  it("renders the real one-click installer at the conflict destination", () => {
    const { parsedRoute, intent, html } = renderRoutedInstaller(
      managedModpackInstallRoute("https://api.mcwok.cn/api/v1/pack/latest"),
    );

    // 冲突时游戏通常已经装着，启动屏必须凭 requested 展开面板，而不是凭「没装游戏」。
    expect(parsedRoute.pathname).toBe("/");
    expect(intent.requested).toBe(true);
    expect(html).toContain("安装 World of Kivotos");
    expect(html).toContain('value="https://api.mcwok.cn/api/v1/pack/latest"');
    // 同上: 深链过来时面板照样展开, 但触发仍归主操作位, 面板只负责说清与改地址。
    expect(html).not.toContain("安装游戏");
  });

  it("prefills the subscribed custom pointer and keeps the built-in reset", () => {
    const pointerUrl = "https://packs.example.test/latest?channel=beta";
    const { intent, html } = renderRoutedInstaller(
      managedModpackInstallRoute(pointerUrl),
    );

    expect(intent.requested).toBe(true);
    expect(html).toContain(`value="${pointerUrl}"`);
    expect(html).toContain("使用WOK 地址");
  });

  it.each([
    "https://user:secret@packs.example.test/latest",
    "https://packs.example.test/latest#release",
  ])("falls back to WOK when the routed pointer is not safe: %s", (pointerUrl) => {
    const { intent, html } = renderRoutedInstaller(
      managedModpackInstallRoute(pointerUrl),
    );

    // 地址被判不可信也照样开面板：人是来装游戏的，只是这条地址不能用。
    expect(intent.requested).toBe(true);
    expect(html).toContain('value="https://api.mcwok.cn/api/v1/pack/latest"');
    expect(html).not.toContain(`value="${pointerUrl}"`);
  });
});
