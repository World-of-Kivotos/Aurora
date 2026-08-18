import { describe, expect, it } from "vitest";
import { modpackOwnerOf } from "./modpack-ownership";
import type { ManagedModpackStatus } from "./modpack-ui";

const FILES = [
  { path: "mods/wok-core.jar", policy: "managed" as const },
  { path: "mods/player-options.jar", policy: "optional" as const },
  { path: "config/wok-client.toml", policy: "seeded" as const },
];

const READY: ManagedModpackStatus = {
  kind: "ready",
  subscription: { pack_id: "wok", pointer_url: "https://example.com/latest" },
  versions: {
    installed_version: "1.9.0",
    latest: {
      pack_id: "wok",
      version: "2.0.0",
      manifest_url: "https://example.com/manifest",
      released_at: "2026-08-17T12:00:00Z",
      note: null,
      min_launcher_version: "0.1.0",
    },
  },
  source: "network",
  checked_at: "1786968300",
};

const READY_WITHOUT_SNAPSHOT: ManagedModpackStatus = {
  ...READY,
  versions: { ...READY.versions, installed_version: null },
};

describe("modpackOwnerOf", () => {
  it("keeps controls disabled until both status and file ownership resolve", () => {
    expect(modpackOwnerOf(undefined, FILES, "wok-core.jar")).toBeNull();
    expect(modpackOwnerOf(READY, undefined, "wok-core.jar")).toBeNull();
    expect(modpackOwnerOf(null, FILES, "wok-core.jar")).toBeNull();
    expect(modpackOwnerOf(READY, null, "wok-core.jar")).toBeNull();
  });

  it("marks only managed snapshot entries as modpack-owned", () => {
    expect(modpackOwnerOf(READY, FILES, "wok-core.jar")).toBe("modpack");
    expect(modpackOwnerOf(READY, FILES, "player-options.jar")).toBe("player");
    expect(modpackOwnerOf(READY, FILES, "local-only.jar")).toBe("player");
    expect(modpackOwnerOf(null, null, "wok-core.jar")).toBe("player");
  });

  it("locks every mod for a managed instance without a successful snapshot", () => {
    expect(modpackOwnerOf(READY_WITHOUT_SNAPSHOT, [], "local-only.jar")).toBeNull();
    expect(
      modpackOwnerOf(
        {
          kind: "unavailable",
          subscription: READY.subscription,
          last_known: null,
          detail: "无法读取远端指针",
        },
        [],
        "local-only.jar",
      ),
    ).toBeNull();
  });

  it("matches the underlying managed file when a mod is disabled", () => {
    expect(modpackOwnerOf(READY, FILES, "wok-core.jar.disabled")).toBe("modpack");
  });

  it("matches Windows case-only aliases with normalized separators", () => {
    const files = [{ path: "MODS\\WÖK-Core.JAR", policy: "managed" as const }];
    expect(modpackOwnerOf(READY, files, "wök-core.jar")).toBe("modpack");
  });
});
