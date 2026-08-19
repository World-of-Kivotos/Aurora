import { describe, expect, it } from "vitest";
import { mockInvoke } from "./ipc-mock";
import type { AppearanceDto, ManagedModpackFile } from "./ipc";
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

// 玻璃模式与背景共用一份外观 DTO。浏览器预览里也必须能真的切换并读回来，
// 否则「不新造存储」这条约定在 mock 分支上就断了，而设置页在浏览器里正是走这一支。
describe("glass mode mock", () => {
  it("默认是磨砂档", async () => {
    const dto = await mockInvoke<AppearanceDto>("get_appearance");
    expect(dto.glass).toBe("frost");
  });

  it("切换后立刻回显，并且 get_appearance 也读得到同一个值", async () => {
    const changed = await mockInvoke<AppearanceDto>("set_glass_mode", { glass: "liquid" });
    expect(changed.glass).toBe("liquid");
    await expect(
      mockInvoke<AppearanceDto>("get_appearance").then((d) => d.glass),
    ).resolves.toBe("liquid");

    // 切回来，免得这条用例把状态漏给同文件里后跑的用例。
    await mockInvoke<AppearanceDto>("set_glass_mode", { glass: "frost" });
  });

  it("切玻璃模式不动背景与柔化，两个设置项各管各的", async () => {
    const before = await mockInvoke<AppearanceDto>("get_appearance");
    const after = await mockInvoke<AppearanceDto>("set_glass_mode", { glass: "liquid" });
    expect(after.background).toBe(before.background);
    expect(after.veil).toBe(before.veil);
    await mockInvoke<AppearanceDto>("set_glass_mode", { glass: "frost" });
  });
});
