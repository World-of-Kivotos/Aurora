import { describe, expect, it } from "vitest";
import {
  managedModpackInstallIntent,
  managedModpackInstallRoute,
  managedModpackPointerFromSearch,
} from "./modpack-navigation";

const ROUTE_BASE = "https://aurora.local";

describe("managedModpackInstallRoute", () => {
  it("round-trips a custom pointer with nested query data", () => {
    const pointerUrl =
      "https://packs.example.com/latest.json?channel=nightly&pointer=https%3A%2F%2Fcdn.example.com%2Fpack%3Fx%3D1%26pointer%3Dinside";
    const route = managedModpackInstallRoute(pointerUrl);
    const parsedRoute = new URL(route, ROUTE_BASE);

    // 目的地是启动屏：下载页的整合包 tab 已随专用化撤掉，指向它就是死链。
    expect(parsedRoute.pathname).toBe("/");
    expect(parsedRoute.searchParams.get("pointer")).toBe(pointerUrl);
    expect(managedModpackPointerFromSearch(parsedRoute.searchParams)).toBe(
      pointerUrl,
    );
  });

  it("keeps a single outer pointer when the pointer carries its own pointer parameter", () => {
    const pointerUrl =
      "https://packs.example.com/latest?pointer=https://evil.example.com/pack&channel=stable";
    const parsedRoute = new URL(
      managedModpackInstallRoute(pointerUrl),
      ROUTE_BASE,
    );

    // 内层那条 pointer 必须留在编码后的值里，不能在外层多出一条被解读为深链参数。
    expect(parsedRoute.searchParams.getAll("pointer")).toEqual([pointerUrl]);
  });
});

describe("managedModpackInstallIntent", () => {
  it("treats a routed pointer as an explicit request to install", () => {
    const pointerUrl = "https://packs.example.com/latest?channel=beta";
    const parsedRoute = new URL(
      managedModpackInstallRoute(pointerUrl),
      ROUTE_BASE,
    );

    expect(managedModpackInstallIntent(parsedRoute.searchParams)).toEqual({
      requested: true,
      pointerUrl,
    });
  });

  it.each([
    "https://user:secret@packs.example.com/latest",
    "https://packs.example.com/latest#release-2026",
    "javascript:alert(1)",
  ])("still opens the installer for an unsafe routed pointer: %s", (pointerUrl) => {
    const parsedRoute = new URL(
      managedModpackInstallRoute(pointerUrl),
      ROUTE_BASE,
    );

    // 地址不可信只影响预填值，不影响「他确实是来装游戏的」这个判断。
    expect(managedModpackInstallIntent(parsedRoute.searchParams)).toEqual({
      requested: true,
      pointerUrl: null,
    });
  });

  it("stays out of the way on a plain launch screen visit", () => {
    expect(managedModpackInstallIntent(new URLSearchParams())).toEqual({
      requested: false,
      pointerUrl: null,
    });
  });
});

describe("managedModpackPointerFromSearch", () => {
  it("returns a trimmed valid HTTP or HTTPS pointer", () => {
    const searchParams = new URLSearchParams({
      pointer: "  https://packs.example.com/latest.json?channel=stable  ",
    });

    expect(managedModpackPointerFromSearch(searchParams)).toBe(
      "https://packs.example.com/latest.json?channel=stable",
    );

    searchParams.set("pointer", " http://127.0.0.1:8080/latest ");
    expect(managedModpackPointerFromSearch(searchParams)).toBe(
      "http://127.0.0.1:8080/latest",
    );
  });

  it.each([
    ["missing", new URLSearchParams()],
    ["empty", new URLSearchParams({ pointer: "   " })],
    ["JavaScript", new URLSearchParams({ pointer: "javascript:alert(1)" })],
    [
      "fragment",
      new URLSearchParams({
        pointer: "https://packs.example.com/latest.json#release-2026",
      }),
    ],
    [
      "userinfo",
      new URLSearchParams({
        pointer: "https://user:secret@packs.example.com/latest.json",
      }),
    ],
    ["malformed", new URLSearchParams({ pointer: "not a URL" })],
  ])("returns null for a %s pointer", (_label, searchParams) => {
    expect(managedModpackPointerFromSearch(searchParams)).toBeNull();
  });
});
