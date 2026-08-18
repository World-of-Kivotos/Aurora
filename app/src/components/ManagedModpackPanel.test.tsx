import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import {
  ManagedModpackPanel,
  ModpackFileManagement,
  ModpackFileOwnership,
} from "./ManagedModpackPanel";
import { Button } from "./Button";
import { ModpackInstallFlow } from "./ModpackInstallFlow";
import { Download } from "../pages/Download";
import { managedModpackInstallRoute } from "../lib/modpack-navigation";
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
        sync={{ kind: "idle" }}
        onCheck={() => undefined}
        onSync={() => undefined}
        onInstallAsNew={() => undefined}
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
        onInstallAsNew={() => undefined}
      />,
    );

    expect(html).toContain("磁盘空间不足：mods/wok-core-2.0.0.jar");
    expect(html).toContain("本次写入需要 16.0 MB，当前可用 4.0 MB");
    expect(html).toContain("重试同步");
  });

  it("opens one-click installation after a conflict without retrying sync", () => {
    const onCheck = vi.fn();
    const onSync = vi.fn();
    const onInstallAsNew = vi.fn();
    const panel = ManagedModpackPanel({
      status: READY,
      sync: {
        kind: "failed",
        target_version: "2.0.0",
        stage: "resolving_manifest",
        failure: { kind: "conflict", detail: "当前 Minecraft 版本无法原地切换" },
      },
      onCheck,
      onSync,
      onInstallAsNew,
    });
    const html = renderToStaticMarkup(panel);

    expect(html).toContain("整合包状态已变化");
    expect(html).toContain("作为新实例安装");
    expect(html).not.toContain("重新检查");
    expect(html).not.toContain("重试同步");

    const installButton = findButton(panel, "作为新实例安装");
    expect(installButton).not.toBeNull();
    installButton?.props.onClick?.();
    expect(onInstallAsNew).toHaveBeenCalledOnce();
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

describe("ModpackInstallFlow", () => {
  it("offers the built-in pointer and the full one-click install sequence", () => {
    const html = renderToStaticMarkup(
      <ModpackInstallFlow
        builtIn={{ label: "WOK 地址", pointer_url: "https://api.mcwok.cn/api/v1/pack/latest" }}
        state={{ kind: "idle" }}
        onInstall={() => undefined}
      />,
    );

    expect(html).toContain('value="https://api.mcwok.cn/api/v1/pack/latest"');
    expect(html).toContain("读取整合包");
    expect(html).toContain("安装 Minecraft");
    expect(html).toContain("安装加载器");
    expect(html).toContain("同步整合包");
    expect(html).toContain("检查并安装");
  });

  it("renders the real one-click installer at the conflict destination", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={["/download?tab=modpack"]}>
        <Download />
      </MemoryRouter>,
    );

    expect(html).toContain("安装服务器整合包");
    expect(html).toContain("https://api.mcwok.cn/api/v1/pack/latest");
    expect(html).toContain("检查并安装");
  });

  it("prefills the subscribed custom pointer and keeps the built-in reset", () => {
    const pointerUrl = "https://packs.example.test/latest?channel=beta";
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={[managedModpackInstallRoute(pointerUrl)]}>
        <Download />
      </MemoryRouter>,
    );

    expect(html).toContain(`value="${pointerUrl}"`);
    expect(html).toContain("使用WOK 地址");
  });

  it.each([
    "https://user:secret@packs.example.test/latest",
    "https://packs.example.test/latest#release",
  ])("falls back to WOK when the routed pointer is not safe: %s", (pointerUrl) => {
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={[managedModpackInstallRoute(pointerUrl)]}>
        <Download />
      </MemoryRouter>,
    );

    expect(html).toContain('value="https://api.mcwok.cn/api/v1/pack/latest"');
    expect(html).not.toContain(`value="${pointerUrl}"`);
  });
});
