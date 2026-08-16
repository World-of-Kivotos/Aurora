import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  ManagedModpackPanel,
  ModpackFileManagement,
  ModpackFileOwnership,
} from "./ManagedModpackPanel";
import { ModpackInstallFlow } from "./ModpackInstallFlow";
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
  checked_at: "2026-08-17T12:05:00Z",
};

describe("ManagedModpackPanel", () => {
  it("shows installed and available versions with an update action", () => {
    const html = renderToStaticMarkup(
      <ManagedModpackPanel status={READY} sync={{ kind: "idle" }} onCheck={() => undefined} onSync={() => undefined} />,
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
      />,
    );

    expect(html).toContain("磁盘空间不足：mods/wok-core-2.0.0.jar");
    expect(html).toContain("本次写入需要 16.0 MB，当前可用 4.0 MB");
    expect(html).toContain("重试同步");
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

    expect(html).toContain("https://api.mcwok.cn/api/v1/pack/latest");
    expect(html).toContain("读取整合包");
    expect(html).toContain("安装 Minecraft");
    expect(html).toContain("安装加载器");
    expect(html).toContain("同步整合包");
    expect(html).toContain("检查并安装");
  });
});
