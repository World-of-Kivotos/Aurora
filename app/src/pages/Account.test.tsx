// 账户列表的四态渲染与「已保存的离线名」一览。
//
// 被测的是这一页唯一会出错的展示分支：离线账户改成持久保存之后，它与正版/外置账户同列一张表，
// 于是「当前」标记该落在谁身上、哪张卡片才给「设为当前」、一个账户都没有时说什么，全靠这段判定。
// 用 renderToStaticMarkup 而不是 testing-library：项目既有做法（见 ManagedModpackPanel.test.tsx）。

import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AccountBoard, SavedOfflineNames } from "./Account";
import type { AccountDto } from "../lib/ipc";

interface ChipProps {
  children?: ReactNode;
  onClick?: () => void;
}

// 在元素树里按显示文字找到那颗离线名 chip。
function findChip(node: ReactNode, label: string): ReactElement<ChipProps> | null {
  if (!isValidElement<ChipProps>(node)) return null;
  if (node.type === "button" && node.props.children === label) return node;
  for (const child of Children.toArray(node.props.children)) {
    const match = findChip(child, label);
    if (match) return match;
  }
  return null;
}

const MICROSOFT: AccountDto = {
  uuid: "853c80ef3c3749fdaa49938b674adae6",
  name: "Shinoyuki_Miyako",
  account_type: "microsoft",
};

// Steve 的原版离线 UUID，与 Rust 侧 offline_uuid 的参照向量同一个值。
const STEVE: AccountDto = {
  uuid: "5627dd98e6be3c21b8a8e92344183641",
  name: "Steve",
  account_type: "offline",
};

const ALEX: AccountDto = {
  uuid: "b0b1eaba0f6f3a4e9e6a7e2fdd1f6b7c",
  name: "Alex",
  account_type: "offline",
};

function board(props: Partial<Parameters<typeof AccountBoard>[0]> = {}): string {
  return renderToStaticMarkup(
    <AccountBoard
      accounts={[MICROSOFT, STEVE]}
      currentUuid={STEVE.uuid}
      loadError={null}
      onRetry={() => undefined}
      onActivate={() => undefined}
      onRemove={() => undefined}
      {...props}
    />,
  );
}

describe("AccountBoard", () => {
  it("把持久化的离线账户与正版账户列在同一张表里", () => {
    const html = board();

    expect(html).toContain("Shinoyuki_Miyako");
    expect(html).toContain("微软正版");
    expect(html).toContain("Steve");
    expect(html).toContain("离线账户");
    // 两张卡片各自都能删。
    expect(html.match(/删除/g)?.length).toBe(2);
  });

  it("「当前」只标在选中的那一个上，且它自己不再出现「设为当前」", () => {
    const html = board({ accounts: [MICROSOFT, STEVE, ALEX], currentUuid: STEVE.uuid });

    expect(html.match(/当前<\/span>/g)?.length).toBe(1);
    // 三个账户里只有两个不是当前，故只有两颗「设为当前」。
    expect(html.match(/设为当前/g)?.length).toBe(2);
  });

  it("当前账户换人时，标记跟着换到另一张卡片上", () => {
    const onSteve = board({ currentUuid: STEVE.uuid });
    const onMicrosoft = board({ currentUuid: MICROSOFT.uuid });

    // 同一份账户列表，只有选中项不同，渲染结果必须不同——否则「当前」根本没生效。
    expect(onSteve).not.toBe(onMicrosoft);
    // 选中项那张卡片不带「设为当前」：截取各自卡片片段来判定。
    const steveCard = onSteve.slice(onSteve.indexOf("Steve"));
    expect(steveCard).not.toContain("设为当前");
    const microsoftCard = onMicrosoft.slice(
      onMicrosoft.indexOf("Shinoyuki_Miyako"),
      onMicrosoft.indexOf("Steve"),
    );
    expect(microsoftCard).not.toContain("设为当前");
  });

  it("空态给出下一步动作，而不是一片空白", () => {
    const html = board({ accounts: [], currentUuid: null });

    expect(html).toContain("还没有账户");
    expect(html).toContain("用下方入口添加一个开始游戏");
    expect(html).not.toContain("设为当前");
  });

  it("尚未加载完与加载失败是两种不同的呈现", () => {
    const loading = board({ accounts: null, currentUuid: null });
    expect(loading).toContain("正在读取账户…");
    expect(loading).not.toContain("还没有账户");

    const failed = board({
      accounts: null,
      currentUuid: null,
      loadError: "该操作在当前平台不受支持（账户凭据加密仅限 Windows）",
    });
    // 错误优先于「读取中」，且原文照登不加工。
    expect(failed).toContain("读取账户失败");
    expect(failed).toContain("账户凭据加密仅限 Windows");
    expect(failed).toContain("重试");
    expect(failed).not.toContain("正在读取账户");
  });
});

describe("SavedOfflineNames", () => {
  it("列出已保存的离线名供直接选用", () => {
    const html = renderToStaticMarkup(
      <SavedOfflineNames names={["Steve", "Alex"]} onPick={() => undefined} />,
    );

    expect(html).toContain("已保存的离线名");
    expect(html).toContain("Steve");
    expect(html).toContain("Alex");
  });

  it("一个都没保存时整块不渲染", () => {
    const html = renderToStaticMarkup(<SavedOfflineNames names={[]} onPick={() => undefined} />);
    expect(html).toBe("");
  });

  it("点某个名字即把它本人交回调用方填进输入框", () => {
    const onPick = vi.fn();
    // renderToStaticMarkup 渲不出事件，故顺着 React 元素树找到那颗 chip 直接触发它的 onClick——
    // 这样「点 Alex 却填进 Steve」这类接线错误才会被抓住（同项目做法见 ManagedModpackPanel.test.tsx）。
    const tree = SavedOfflineNames({ names: ["Steve", "Alex"], onPick });
    const alex = findChip(tree, "Alex");
    expect(alex).not.toBeNull();
    alex?.props.onClick?.();

    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onPick).toHaveBeenCalledWith("Alex");
  });
});
