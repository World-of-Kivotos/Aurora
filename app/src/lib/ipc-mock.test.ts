import { describe, expect, it } from "vitest";
import { mockInvoke } from "./ipc-mock";
import type { ManagedModpackFile } from "./ipc";
import type { CheckedManagedModpackStatus } from "./modpack-ui";

describe("managed modpack browser mock", () => {
  it("labels its managed status as browser preview data", async () => {
    const status = await mockInvoke<CheckedManagedModpackStatus | null>(
      "managed_modpack_status",
      { versionId: "World of Kivotos 2.0 beta" },
    );

    expect(status?.subscription.pack_id).toBe("wok-browser-preview");
    expect(status?.kind).toBe("ready");
    if (status?.kind === "ready") {
      expect(status.versions.latest.note).toContain("浏览器模拟状态");
    }
  });

  it("does not mark ordinary preview instances as managed", async () => {
    await expect(
      mockInvoke<CheckedManagedModpackStatus | null>("managed_modpack_status", {
        versionId: "1.21.4",
      }),
    ).resolves.toBeNull();
  });

  it("returns explicit managed and seeded ownership policies", async () => {
    const files = await mockInvoke<ManagedModpackFile[] | null>("managed_modpack_files", {
      versionId: "World of Kivotos 2.0 beta",
    });
    expect(files).toEqual([
      { path: "mods/sodium-fabric-0.6.0.jar", policy: "managed" },
      { path: "config/wok-client.toml", policy: "seeded" },
    ]);
  });

  it("disables independent platform updates for the managed preview instance", async () => {
    await expect(
      mockInvoke<unknown[]>("check_updates", { versionId: "World of Kivotos 2.0 beta" }),
    ).resolves.toEqual([]);
  });

  it.each([
    ["sync_managed_modpack", { versionId: "World of Kivotos 2.0 beta", targetVersion: "2.0.0-preview" }],
    ["install_managed_modpack", { pointerUrl: "https://api.mcwok.cn/api/v1/pack/latest" }],
  ])("rejects %s instead of pretending to write real files", async (command, args) => {
    await expect(mockInvoke<never>(command, args)).rejects.toMatchObject({
      stage: "resolving_manifest",
      failure: {
        kind: "filesystem",
        file_path: "<browser-preview>",
        detail: expect.stringContaining("不会执行真实安装或写入磁盘"),
      },
    });
  });
});
