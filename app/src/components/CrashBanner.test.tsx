import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CrashReport } from "../lib/ipc";
import type { ModpackFileOwner } from "../lib/modpack-ui";
import { CrashBanner } from "./CrashBanner";

interface CapturedButtonProps {
  children?: ReactNode;
  disabled?: boolean;
  onClick?: () => void;
}

const doubles = vi.hoisted(() => ({
  buttons: [] as CapturedButtonProps[],
  setModEnabled: vi.fn(),
  toast: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  setModEnabled: doubles.setModEnabled,
}));

vi.mock("./Toast", () => ({
  useToast: () => ({ toast: doubles.toast }),
}));

vi.mock("./Button", () => ({
  Button: (props: CapturedButtonProps) => {
    doubles.buttons.push(props);
    return createElement("button", { disabled: props.disabled }, props.children);
  },
}));

const reportFor = (fileName: string): CrashReport => ({
  diagnoses: [],
  suspects: [{ mod_id: "example", file_name: fileName }],
  log_path: null,
});

function renderBanner(
  ownerOf: (fileName: string) => ModpackFileOwner | null,
  report: CrashReport = reportFor("example.jar"),
): string {
  return renderToStaticMarkup(
    <CrashBanner
      report={report}
      versionId="test-instance"
      onDismiss={() => undefined}
      onOpenDetail={() => undefined}
      ownerOf={ownerOf}
      onPhoto={false}
    />,
  );
}

function findDisableButton(): CapturedButtonProps | undefined {
  return doubles.buttons.find((button) => button.children === "禁用它");
}

describe("CrashBanner file ownership", () => {
  beforeEach(() => {
    doubles.buttons.length = 0;
    doubles.setModEnabled.mockReset();
    doubles.setModEnabled.mockResolvedValue("example.jar.disabled");
    doubles.toast.mockReset();
  });

  it("does not offer or call disable for a managed file", () => {
    const html = renderBanner(() => "modpack");

    expect(html).toContain("由整合包统一维护，不能单独禁用");
    expect(html).not.toContain("禁用它");
    expect(findDisableButton()).toBeUndefined();
    expect(doubles.setModEnabled).not.toHaveBeenCalled();
  });

  it("does not offer or call disable while ownership is unresolved", () => {
    const html = renderBanner(() => null);

    expect(html).toContain("文件归属尚未确认，暂不提供禁用操作");
    expect(html).not.toContain("禁用它");
    expect(findDisableButton()).toBeUndefined();
    expect(doubles.setModEnabled).not.toHaveBeenCalled();
  });

  it("resolves each suspect independently and disables only the player-owned file", () => {
    const report: CrashReport = {
      diagnoses: [],
      suspects: [
        { mod_id: "managed", file_name: "managed.jar" },
        { mod_id: "unknown", file_name: "unknown.jar" },
        { mod_id: "player", file_name: "player.jar" },
      ],
      log_path: null,
    };
    const owners = new Map<string, ModpackFileOwner>([
      ["managed.jar", "modpack"],
      ["player.jar", "player"],
    ]);

    const html = renderBanner((fileName) => owners.get(fileName) ?? null, report);

    const disableButton = findDisableButton();
    expect(html).toContain("由整合包统一维护，不能单独禁用");
    expect(html).toContain("文件归属尚未确认，暂不提供禁用操作");
    expect(doubles.buttons.filter((button) => button.children === "禁用它")).toHaveLength(1);
    expect(disableButton).toBeDefined();
    disableButton?.onClick?.();

    expect(doubles.setModEnabled).toHaveBeenCalledOnce();
    expect(doubles.setModEnabled).toHaveBeenCalledWith("test-instance", "player.jar", false);
  });

  it("rechecks ownership before calling the disable IPC", () => {
    let owner: ModpackFileOwner | null = "player";
    renderBanner(() => owner);
    const disableButton = findDisableButton();

    owner = null;
    disableButton?.onClick?.();

    expect(doubles.setModEnabled).not.toHaveBeenCalled();
  });
});
