import { describe, expect, it } from "vitest";
import {
  managedModpackInstallRoute,
  managedModpackPointerFromSearch,
} from "./modpack-navigation";

describe("managedModpackInstallRoute", () => {
  it("round-trips a custom pointer with nested query data", () => {
    const pointerUrl =
      "https://packs.example.com/latest.json?channel=nightly&tab=version&redirect=https%3A%2F%2Fcdn.example.com%2Fpack%3Fx%3D1%26tab%3Dinside";
    const route = managedModpackInstallRoute(pointerUrl);
    const parsedRoute = new URL(route, "https://aurora.local");

    expect(parsedRoute.pathname).toBe("/download");
    expect(parsedRoute.searchParams.getAll("tab")).toEqual(["modpack"]);
    expect(parsedRoute.searchParams.get("pointer")).toBe(pointerUrl);
    expect(managedModpackPointerFromSearch(parsedRoute.searchParams)).toBe(
      pointerUrl,
    );
  });

  it("keeps the outer tab fixed when the pointer contains tab parameters", () => {
    const pointerUrl =
      "https://packs.example.com/latest?tab=version&tab=instances&channel=stable";
    const parsedRoute = new URL(
      managedModpackInstallRoute(pointerUrl),
      "https://aurora.local",
    );

    expect(parsedRoute.searchParams.getAll("tab")).toEqual(["modpack"]);
    expect(parsedRoute.searchParams.get("pointer")).toBe(pointerUrl);
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
