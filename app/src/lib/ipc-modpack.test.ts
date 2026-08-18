import { beforeEach, describe, expect, it, vi } from "vitest";

const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("./tauri-bridge", () => bridge);

import {
  CORE_EVENT,
  installManagedModpack,
  managedModpackFiles,
  managedModpackStatus,
  onCoreEvent,
  syncManagedModpack,
} from "./ipc";

describe("managed modpack IPC", () => {
  beforeEach(() => {
    bridge.invoke.mockReset();
    bridge.listen.mockReset();
  });

  it("uses Tauri camelCase argument keys for status and ownership", async () => {
    bridge.invoke.mockResolvedValueOnce(null).mockResolvedValueOnce([]);

    await managedModpackStatus("forge-47.4.16");
    await managedModpackFiles("forge-47.4.16");

    expect(bridge.invoke).toHaveBeenNthCalledWith(1, "managed_modpack_status", {
      versionId: "forge-47.4.16",
    });
    expect(bridge.invoke).toHaveBeenNthCalledWith(2, "managed_modpack_files", {
      versionId: "forge-47.4.16",
    });
  });

  it("keeps sync and install commands distinct", async () => {
    bridge.invoke
      .mockResolvedValueOnce({
        installed_version: "2.0.0",
        downloaded_files: 4,
        deleted_files: 1,
        kept_files: 12,
      })
      .mockResolvedValueOnce({ instance_id: "forge-47.4.16", installed_version: "2.0.0" });

    await syncManagedModpack("forge-47.4.16", "2.0.0");
    await installManagedModpack("https://api.mcwok.cn/api/v1/pack/latest");

    expect(bridge.invoke).toHaveBeenNthCalledWith(
      1,
      "sync_managed_modpack",
      expect.objectContaining({
        versionId: "forge-47.4.16",
        targetVersion: "2.0.0",
        operationId: expect.stringMatching(/^modpack-/),
      }),
    );
    expect(bridge.invoke).toHaveBeenNthCalledWith(
      2,
      "install_managed_modpack",
      expect.objectContaining({
        pointerUrl: "https://api.mcwok.cn/api/v1/pack/latest",
        operationId: expect.stringMatching(/^modpack-/),
      }),
    );
  });

  it("forwards the structured modpack progress payload", async () => {
    const progress = {
      stage: "downloading_files" as const,
      completed_files: 3,
      total_files: 8,
      downloaded_bytes: 4096,
      total_bytes: 16384,
      current_file: null,
    };
    bridge.listen.mockImplementationOnce(async (eventName, listener) => {
      listener({ event: eventName, payload: { kind: "modpack_sync", operation_id: "op-1", progress } });
      return () => undefined;
    });
    const handler = vi.fn();

    await onCoreEvent(handler);

    expect(bridge.listen).toHaveBeenCalledWith(CORE_EVENT, expect.any(Function));
    expect(handler).toHaveBeenCalledWith({ kind: "modpack_sync", operation_id: "op-1", progress });
  });

  it("delivers only progress correlated with the invoking operation", async () => {
    const unlisten = vi.fn();
    let eventListener: ((event: { payload: unknown }) => void) | null = null;
    bridge.listen.mockImplementationOnce(async (_eventName, listener) => {
      eventListener = listener;
      return unlisten;
    });
    bridge.invoke.mockImplementationOnce(async (_command, args) => {
      const progress = {
        stage: "writing_snapshot",
        completed_files: 1,
        total_files: 1,
        downloaded_bytes: 1024,
        total_bytes: 1024,
        current_file: ".aurora/modpack-applied.json",
      };
      const emit = eventListener as unknown as (event: { payload: unknown }) => void;
      emit({ payload: { kind: "modpack_sync", operation_id: "another-operation", progress } });
      emit({ payload: { kind: "modpack_sync", operation_id: args.operationId, progress } });
      return { installed_version: "2.0.0", downloaded_files: 1, deleted_files: 0, kept_files: 0 };
    });
    const handler = vi.fn();

    await syncManagedModpack("forge-47.4.16", "2.0.0", handler);

    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith(expect.objectContaining({ stage: "writing_snapshot" }));
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
