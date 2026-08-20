// 启动屏主操作键的安装进度折算。
//
// 被测的是「四步各自的百分比从哪来」这条业务规则本身：第四步走整合包同步的字节数，
// 前三步走下载器的文件计数，两者都没有时总进度停在本步起点、并借后端那句中文阶段说明交代现场。
// 这段折算只在一次真实安装的长流程里才跑得到，所以它必须单独可测。

import { describe, expect, it } from "vitest";
import { installProgressView, type InstallFeed } from "./Home";
import type { ModpackSyncProgress, ModpackSyncStage } from "../lib/modpack-ui";

function progressOf(stage: ModpackSyncStage, patch: Partial<ModpackSyncProgress> = {}): ModpackSyncProgress {
  return {
    stage,
    completed_files: 0,
    total_files: 0,
    downloaded_bytes: 0,
    total_bytes: null,
    current_file: null,
    download_speed: null,
    ...patch,
  };
}

function feedOf(stage: ModpackSyncStage, patch: Partial<InstallFeed> = {}): InstallFeed {
  return { stage, message: null, download: null, ...patch };
}

describe("installProgressView", () => {
  it("第四步按字节算：步内百分比与总进度都是真数，速度直接取后端那一帧", () => {
    const view = installProgressView(
      progressOf("downloading_files", {
        completed_files: 30,
        total_files: 120,
        downloaded_bytes: 60 * 1024 * 1024,
        total_bytes: 120 * 1024 * 1024,
        download_speed: 8 * 1024 * 1024,
      }),
      feedOf("downloading_files"),
    );

    expect(view.counted).toBe(true);
    // 第四步占 0.62~0.96，跑到一半即 0.62 + 0.34 x 0.5。
    expect(view.overall).toBeCloseTo(0.79, 5);
    // 速度是后端下载引擎 EWMA 平滑后的 download_speed 原值，前端不再按字节增量推算。
    expect(view.rate).toBe("8.0 MB/s");
    expect(view.activity).toBe("下载并校验文件 50% · 60.0 MB / 120.0 MB · 8.0 MB/s");
  });

  it("第四步的速度只认 download_speed：这一步的 download 事件残帧不许顶上来", () => {
    const view = installProgressView(
      progressOf("downloading_files", {
        completed_files: 30,
        total_files: 120,
        downloaded_bytes: 60 * 1024 * 1024,
        total_bytes: 120 * 1024 * 1024,
        download_speed: 2 * 1024 * 1024,
      }),
      feedOf("downloading_files", {
        download: { total: 120, finished: 30, bytes: 4096, speed: 99 * 1024 * 1024 },
      }),
    );

    expect(view.rate).toBe("2.0 MB/s");
  });

  it("后端报 null 与报 0 都不画速度那一格，但两者不是同一件事", () => {
    // null = 这一步压根没有下载行为（删文件 / 写快照）；0 = 下载在跑，只是这一瞬没有字节进来。
    // 界面上都不该出现一个「0 B/s」把玩家吓成卡死，故两种输入的 rate 都是 null。
    for (const download_speed of [null, 0]) {
      const view = installProgressView(
        progressOf("writing_snapshot", {
          completed_files: 1,
          total_files: 1,
          downloaded_bytes: 120 * 1024 * 1024,
          total_bytes: 120 * 1024 * 1024,
          download_speed,
        }),
        feedOf("writing_snapshot"),
      );

      expect(view.rate).toBeNull();
      expect(view.activity).toBe("保存同步快照 100% · 120.0 MB / 120.0 MB");
    }
  });

  it("前三步接上下载器的文件计数后，同样算得出步内百分比与速度", () => {
    const view = installProgressView(
      progressOf("installing_minecraft"),
      feedOf("installing_minecraft", {
        message: "开始安装原版 1.20.1",
        download: { total: 24, finished: 12, bytes: 4096, speed: 3 * 1024 * 1024 },
      }),
    );

    expect(view.counted).toBe(true);
    // 装 Minecraft 占 0.04~0.46，跑到一半即 0.04 + 0.42 x 0.5。
    expect(view.overall).toBeCloseTo(0.25, 5);
    expect(view.activity).toBe("安装 Minecraft 50% · 12/24 个文件 · 3.0 MB/s");
  });

  it("这一步没有任何计数时：总进度停在本步起点，现场退回后端那句阶段说明", () => {
    const view = installProgressView(
      progressOf("installing_minecraft"),
      feedOf("installing_minecraft", { message: "开始安装原版 1.20.1" }),
    );

    expect(view.counted).toBe(false);
    expect(view.overall).toBeCloseTo(0.04, 5);
    expect(view.activity).toBe("安装 Minecraft · 开始安装原版 1.20.1");
  });

  it("上一步的文案与文件计数不许串到下一步上", () => {
    const view = installProgressView(
      progressOf("installing_loader"),
      feedOf("installing_minecraft", {
        message: "原版 1.20.1 安装完成：库 43 / 资源 3402 / natives 12",
        download: { total: 24, finished: 24, bytes: 4096, speed: 1024 },
      }),
    );

    expect(view.counted).toBe(false);
    expect(view.overall).toBeCloseTo(0.46, 5);
    expect(view.activity).toBe("安装加载器");
  });

  it("四步的起点严格递增，总进度不会因为换步而回退", () => {
    const stages: ModpackSyncStage[] = [
      "resolving_manifest",
      "installing_minecraft",
      "installing_loader",
      "downloading_files",
      "deleting_files",
      "writing_snapshot",
    ];
    const heads = stages.map(
      (stage) => installProgressView(progressOf(stage), feedOf(stage)).overall,
    );

    expect(heads).toEqual([...heads].sort((a, b) => a - b));
    expect(new Set(heads).size).toBe(stages.length);
    // 收尾阶段跑满即 100%，不留一个永远差一点的进度条。
    const done = installProgressView(
      progressOf("writing_snapshot", { completed_files: 8, total_files: 8 }),
      feedOf("writing_snapshot"),
    );
    expect(done.overall).toBeCloseTo(1, 5);
  });
});
