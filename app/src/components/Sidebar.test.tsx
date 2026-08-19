// 侧栏游戏列表的契约。四件事值得钉死，因为它们坏掉都不会报错、只会悄悄变成另一种产品：
// 1) 未上线的游戏必须不可点——它一旦被写成 NavLink，点进去就是启动屏，用户以为 Arena 能玩了；
// 2) 「World of Kivotos」是启动屏的唯一入口（原「主页」项已删），它若丢了当前态，
//    侧栏就再没有任何地方指示"你正在这一屏"；
// 3) 排版把名字切成了眉标 + 主名两截，无障碍名必须仍是完整的一句，否则读屏念出来是断的；
// 4) 两个条目的主名必须真的不同——共享的那半句「World of」被降成了眉标，
//    要是主名也复制成一样的，侧栏就成了两行看不出区别的东西。

import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { Sidebar } from "./Sidebar";

const WOK_FULL = "World of Kivotos";
const ARENA_TITLE = "Kivotos : Arena";

function renderAt(pathname: string): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[pathname]}>
      <Sidebar onPhoto={false} />
    </MemoryRouter>,
  );
}

/** 取 markup 里所有 <a> 的开标签到闭合，用于判定某段文字是否落在链接内。 */
function anchors(markup: string): string[] {
  return markup.split("<a ").slice(1).map((chunk) => chunk.split("</a>")[0]);
}

describe("Sidebar 游戏列表", () => {
  it("未上线的 Arena 渲染成不可点的行, 不是链接", () => {
    const markup = renderAt("/");

    expect(markup).toContain(ARENA_TITLE);
    expect(markup).toContain('aria-disabled="true"');
    expect(anchors(markup).some((anchor) => anchor.includes(ARENA_TITLE))).toBe(false);
    expect(markup).toContain("敬请期待");
  });

  it("World of Kivotos 指向启动屏, 且已无独立的「主页」导航项", () => {
    const markup = renderAt("/");
    const wokAnchor = anchors(markup).find((anchor) => anchor.includes('aria-label="' + WOK_FULL + '"'));

    expect(wokAnchor).toBeDefined();
    expect(wokAnchor).toContain('href="/"');
    // 删掉的那一项不能被顺手加回来: 同一条路由挂两个入口, 竖规会不知道该落在哪一行。
    expect(markup).not.toContain(">主页<");
  });

  it("眉标被 CSS 转成全大写, 故完整名字必须由 aria-label 兜住", () => {
    const markup = renderAt("/");

    // 可见文本里眉标与主名分属两个元素, 且眉标靠 CSS uppercase 显示,
    // 完整且保留原大小写的那一句只存在于 aria-label。
    expect(markup).toContain('aria-label="' + WOK_FULL + '"');
    expect(markup).not.toContain(">" + WOK_FULL + "<");
    expect(markup).toContain("uppercase");
    // 眉标的 DOM 文本必须是原大小写: 若为了省事直接写死 "WORLD OF",
    // aria-label 与可见文本就会各说各话, 而且再也无法还原成正常大小写。
    expect(markup).toContain(">World of<");
    expect(markup).not.toContain(">WORLD OF<");
  });

  it("在启动屏时游戏行取当前态, 切到别的页就交出去", () => {
    const onHome = renderAt("/");
    const onVersions = renderAt("/versions");

    // 朱红竖规全局只有一道, 在启动屏时它属于游戏行。
    expect(onHome.split("bg-accent").length - 1).toBe(1);
    const homeWokAnchor = anchors(onHome).find((anchor) => anchor.includes('aria-label="' + WOK_FULL + '"'));
    expect(homeWokAnchor).toContain("bg-accent");
    expect(homeWokAnchor).toContain("font-extrabold");

    // 换页后竖规仍只有一道, 但已不在游戏行里。
    expect(onVersions.split("bg-accent").length - 1).toBe(1);
    const versionsWokAnchor = anchors(onVersions).find((anchor) => anchor.includes('aria-label="' + WOK_FULL + '"'));
    expect(versionsWokAnchor).not.toContain("bg-accent");
    expect(versionsWokAnchor).not.toContain("font-extrabold");
  });

  it("两个条目共享眉标但主名不同, 且不再有任何图标", () => {
    const markup = renderAt("/");

    expect(markup.split(">World of<").length - 1).toBe(2);
    expect(markup).toContain(">Kivotos<");
    expect(markup).toContain(">" + ARENA_TITLE + "<");
    // 图标已按要求撤掉, 别再加回来。
    expect(markup).not.toContain("<img");
  });
});
