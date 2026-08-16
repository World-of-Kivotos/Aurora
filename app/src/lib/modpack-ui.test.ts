import { describe, expect, it } from "vitest";
import {
  canRemoveModpackFile,
  formatModpackBytes,
  modpackUpdateAvailable,
  presentSyncFailure,
  syncProgressRatio,
  validateModpackPointerUrl,
  type ModpackSyncFailure,
} from "./modpack-ui";

describe("validateModpackPointerUrl", () => {
  it.each([
    ["", "请输入整合包地址"],
    ["not a url", "整合包地址不是有效 URL"],
    ["file:///D:/pack/latest.json", "整合包地址只支持 HTTP 或 HTTPS"],
    ["https://user:secret@example.com/pack/latest", "整合包地址不能包含账号或密码"],
  ])("rejects %j with a specific reason", (value, reason) => {
    expect(validateModpackPointerUrl(value)).toBe(reason);
  });

  it.each([
    "https://api.mcwok.cn/api/v1/pack/latest",
    " http://127.0.0.1:8080/api/v1/pack/latest ",
  ])("accepts an HTTP pointer URL: %s", (value) => {
    expect(validateModpackPointerUrl(value)).toBeNull();
  });
});

describe("syncProgressRatio", () => {
  it("uses verified byte progress before file count", () => {
    expect(
      syncProgressRatio({
        stage: "downloading_files",
        completed_files: 9,
        total_files: 10,
        downloaded_bytes: 25,
        total_bytes: 100,
        current_file: "mods/wok.jar",
      }),
    ).toBe(0.25);
  });

  it("falls back to file count and clamps interrupted progress", () => {
    expect(
      syncProgressRatio({
        stage: "deleting_files",
        completed_files: 3,
        total_files: 4,
        downloaded_bytes: 0,
        total_bytes: null,
        current_file: null,
      }),
    ).toBe(0.75);
    expect(
      syncProgressRatio({
        stage: "writing_snapshot",
        completed_files: 11,
        total_files: 10,
        downloaded_bytes: 0,
        total_bytes: null,
        current_file: ".aurora/modpack-applied.json",
      }),
    ).toBe(1);
  });

  it("stays bounded for randomized byte counters", () => {
    let seed = 0x5f3759df;
    for (let index = 0; index < 128; index += 1) {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      const total = (seed % 10_000) + 1;
      seed = (seed * 1664525 + 1013904223) >>> 0;
      const downloaded = (seed % 20_000) - 5_000;
      const ratio = syncProgressRatio({
        stage: "downloading_files",
        completed_files: 0,
        total_files: 0,
        downloaded_bytes: downloaded,
        total_bytes: total,
        current_file: "mods/randomized.jar",
      });
      expect(ratio).toBeGreaterThanOrEqual(0);
      expect(ratio).toBeLessThanOrEqual(1);
    }
  });
});

describe("presentSyncFailure", () => {
  const cases: { failure: ModpackSyncFailure; title: string; action: string }[] = [
    {
      failure: { kind: "network", file_path: "mods/a.jar", detail: "连接超时" },
      title: "下载失败：mods/a.jar",
      action: "检查网络连接或代理设置后重试。已校验完成的文件不会重复下载。",
    },
    {
      failure: {
        kind: "checksum_mismatch",
        file_path: "mods/b.jar",
        expected_sha1: "expected",
        actual_sha1: "actual",
      },
      title: "校验不符：mods/b.jar",
      action: "重试下载；若仍然不符，请把文件名和校验值发给整合包维护者。",
    },
    {
      failure: {
        kind: "disk_full",
        file_path: "mods/c.jar",
        required_bytes: 2 * 1024 * 1024,
        available_bytes: 512 * 1024,
      },
      title: "磁盘空间不足：mods/c.jar",
      action: "释放实例所在磁盘的空间后重试。同步尚未删除任何旧文件。",
    },
    {
      failure: { kind: "permission_denied", file_path: "mods/d.jar", detail: "文件正被占用" },
      title: "无法写入：mods/d.jar",
      action: "关闭正在占用该文件的程序，确认目录可写后重试。",
    },
    {
      failure: {
        kind: "snapshot_write",
        file_path: ".aurora/modpack-applied.json",
        detail: "拒绝访问",
      },
      title: "无法保存同步快照：.aurora/modpack-applied.json",
      action: "确认实例目录可写后重试。快照未写入前，本次同步不会被标记为完成。",
    },
  ];

  it.each(cases)("keeps the failing file and actionable guidance for $failure.kind", ({ failure, title, action }) => {
    const presentation = presentSyncFailure(failure);
    expect(presentation.title).toBe(title);
    expect(presentation.action).toBe(action);
    expect(presentation.reason.length).toBeGreaterThan(0);
  });

  it("reports both expected and actual hashes", () => {
    const presentation = presentSyncFailure({
      kind: "checksum_mismatch",
      file_path: "mods/wok-core.jar",
      expected_sha1: "aa11",
      actual_sha1: "bb22",
    });
    expect(presentation.reason).toBe("期望 SHA-1 aa11，实际为 bb22。");
  });
});

describe("managed pack policy", () => {
  it("only exposes removal for player-installed files", () => {
    expect(canRemoveModpackFile("modpack")).toBe(false);
    expect(canRemoveModpackFile("player")).toBe(true);
  });

  it("compares the installed snapshot with the published version", () => {
    const latest = {
      pack_id: "wok",
      version: "2.0.0",
      manifest_url: "https://api.mcwok.cn/api/v1/pack/manifest/2.0.0",
      released_at: "2026-08-17T12:00:00Z",
      note: null,
      min_launcher_version: "0.3.0",
    };
    expect(modpackUpdateAvailable({ installed_version: "1.9.0", latest })).toBe(true);
    expect(modpackUpdateAvailable({ installed_version: "2.0.0", latest })).toBe(false);
    expect(modpackUpdateAvailable({ installed_version: null, latest })).toBe(true);
  });
});

describe("formatModpackBytes", () => {
  it.each([
    [0, "0 B"],
    [1023, "1023 B"],
    [1024, "1.0 KB"],
    [1024 * 1024, "1.0 MB"],
    [1024 * 1024 * 1024, "1.00 GB"],
  ])("formats the boundary %d", (bytes, expected) => {
    expect(formatModpackBytes(bytes)).toBe(expected);
  });

  it.each([-1, Number.NaN, Number.POSITIVE_INFINITY])("rejects invalid byte count %s", (bytes) => {
    expect(() => formatModpackBytes(bytes)).toThrow(RangeError);
  });
});
