// 侧栏游戏列表的契约。六件事值得钉死，因为它们坏掉都不会报错、只会悄悄变成另一种产品：
// 1) 未上线的游戏必须不可点——它一旦被写成 NavLink，点进去就是启动屏，用户以为 Arena 能玩了；
// 2) 「World of Kivotos」是启动屏的唯一入口（原「主页」项已删），它若丢了当前态，
//    侧栏就再没有任何地方指示"你正在这一屏"；
// 3) 排版把名字切成了眉标 + 主名两截，无障碍名必须仍是完整的一句，否则读屏念出来是断的；
// 4) 两个条目的主名必须真的不同——共享的那半句「World of」被降成了眉标，
//    要是主名也复制成一样的，侧栏就成了两行看不出区别的东西；
// 5) 背景图铺满全站后，任何承载文字的块都压在照片上，材质类一旦漏挂就是「没有底的浮层」——
//    这在纯纸底的测试环境里看不出来，只有真机开了背景图才会暴露，所以必须由断言兜住；
// 6) 卷宗页入口（游戏行右侧那枚箭头）只在实例真的装出来之后才存在——实例是装受管整合包的产物，
//    没装就把入口挂出去，玩家点进去只会撞上一页空态。它的出现与隐藏都得由断言守住；
// 7) 箭头与游戏行主体必须是两个并列的可点元素，不是「链接里套链接」——嵌套可点物是无效 HTML，
//    点击归属由浏览器自行发挥，读屏也会把两个可及名读成一团。这类结构错误渲染出来完全看不出来，
//    只有断言能拦住。

import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { Sidebar, SidebarView } from "./Sidebar";

const WOK_FULL = "World of Kivotos";
const ARENA_TITLE = "Kivotos : Arena";

function renderAt(pathname: string, onPhoto = false): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[pathname]}>
      <Sidebar onPhoto={onPhoto} />
    </MemoryRouter>,
  );
}

/**
 * 直接渲染纯视图层，用来指定实例是否已就位。走 Sidebar 本身探不到这一半：
 * 实例状态由一次 IPC 探测得来，而静态渲染不跑副作用，探测结果永远停在「未就位」。
 */
function renderView(pathname: string, instanceReady: boolean): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[pathname]}>
      <SidebarView onPhoto={false} instanceReady={instanceReady} />
    </MemoryRouter>,
  );
}

/** 取 markup 里所有 <a> 的开标签到闭合，用于判定某段文字是否落在链接内。 */
function anchors(markup: string): string[] {
  return markup.split("<a ").slice(1).map((chunk) => chunk.split("</a>")[0]);
}

/**
 * 含 marker 的那个 <a> 从开标签到最近一个 </a> 的原文。
 *
 * 不能用上面的 anchors()：它按 "<a " 切分，嵌套的内层链接恰好会被当成分隔符吃掉，
 * 于是「链接里套链接」这种结构错误在切分结果里反而看不见。这里按下标切原文，
 * 一旦嵌套，内层那个开标签就会原封不动留在结果里，断言才拦得住。
 */
function anchorBlock(markup: string, marker: string): string {
  const at = markup.indexOf(marker);
  if (at < 0) throw new Error(`markup 里找不到 ${marker}`);
  return markup.slice(markup.lastIndexOf("<a ", at), markup.indexOf("</a>", at));
}

/** 该段里除了它自己的开标签外，不得再有第二个可点元素。 */
function expectNoNestedInteractive(block: string): void {
  expect(block.slice("<a ".length)).not.toContain("<a ");
  expect(block).not.toContain("<button");
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
    const onDownload = renderAt("/download");

    // 朱红竖规全局只有一道, 在启动屏时它属于游戏行。
    expect(onHome.split("bg-accent").length - 1).toBe(1);
    const homeWokAnchor = anchors(onHome).find((anchor) => anchor.includes('aria-label="' + WOK_FULL + '"'));
    expect(homeWokAnchor).toContain("bg-accent");
    expect(homeWokAnchor).toContain("font-extrabold");

    // 换页后竖规仍只有一道, 但已不在游戏行里。
    expect(onDownload.split("bg-accent").length - 1).toBe(1);
    const downloadWokAnchor = anchors(onDownload).find((anchor) => anchor.includes('aria-label="' + WOK_FULL + '"'));
    expect(downloadWokAnchor).not.toContain("bg-accent");
    expect(downloadWokAnchor).not.toContain("font-extrabold");
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

describe("Sidebar 卷宗页入口", () => {
  const MANAGE_LABEL = 'aria-label="管理 ' + WOK_FULL + '"';

  it("实例还没装出来时游戏行里不长出那枚箭头", () => {
    const markup = renderView("/", false);

    expect(markup).not.toContain(MANAGE_LABEL);
    expect(markup).not.toContain(">管理<");
    expect(markup).not.toContain('href="/instance"');
    // 主体那一条链接照旧, 少掉的只有箭头。
    expect(anchors(markup)).toHaveLength(4);
    // 探测是异步的, 静态渲染那一帧还没有结果 —— 那一帧也必须是「没有入口」,
    // 否则入口会先冒出来再消失, 玩家正好点上就撞进空态。
    expect(renderAt("/")).not.toContain('href="/instance"');
  });

  it("实例就位后箭头出现在游戏行内, 指向 /instance 且不画方框", () => {
    const markup = renderView("/", true);
    const manageAnchor = anchors(markup).find((anchor) => anchor.includes(MANAGE_LABEL));

    expect(manageAnchor).toBeDefined();
    expect(manageAnchor).toContain('href="/instance"');
    // 可见的只有一枚箭头, 图形本身对读屏不出声(aria-hidden), 去哪由 aria-label 说清。
    expect(manageAnchor).toContain("<svg");
    expect(manageAnchor).toContain('aria-hidden="true"');
    // 与其它可点行同一种手感: 无控件底、悬停才浮墨, 且有自己的焦点环。
    expect(manageAnchor).not.toContain("surface-control");
    expect(manageAnchor).toContain("hover:bg-ink/6");
    expect(manageAnchor).toContain("focus-visible:outline-accent");
    // 仍在启动屏, 当前态属于游戏行, 竖规全局只有一道。
    expect(markup.split("bg-accent").length - 1).toBe(1);
    expect(anchors(markup).find((a) => a.includes('aria-label="' + WOK_FULL + '"'))).toContain(
      "bg-accent",
    );
  });

  it("箭头与游戏行主体是两个并列的可点元素, 不是链接里套链接", () => {
    const markup = renderView("/", true);
    const mainBlock = anchorBlock(markup, 'aria-label="' + WOK_FULL + '"');
    const arrowBlock = anchorBlock(markup, MANAGE_LABEL);

    // 各自是一个独立的 <a>, 各自有自己的可及名与目的地。
    expect(mainBlock).toContain('href="/"');
    expect(mainBlock).toContain("focus-visible:outline-accent");
    expect(arrowBlock).toContain('href="/instance"');
    expect(arrowBlock).toContain("focus-visible:outline-accent");

    // 谁都不许把对方(或任何 button)套进自己肚子里。
    expectNoNestedInteractive(mainBlock);
    expectNoNestedInteractive(arrowBlock);

    // 并列的形态还得体现在数量上: 这一屏只该有主体、箭头, 外加账户/下载/设置三条导航。
    expect(anchors(markup)).toHaveLength(5);
  });

  it("进了卷宗页由箭头接过当前态, 竖规仍只有一道", () => {
    const markup = renderView("/instance", true);
    const manageAnchor = anchors(markup).find((anchor) => anchor.includes(MANAGE_LABEL));
    const wokAnchor = anchors(markup).find((anchor) =>
      anchor.includes('aria-label="' + WOK_FULL + '"'),
    );

    expect(markup.split("bg-accent").length - 1).toBe(1);
    expect(manageAnchor).toContain("bg-accent");
    // 游戏行按 end 匹配, 到了子页面就该把当前态交出去, 否则两行会同时读成「你在这」。
    expect(wokAnchor).not.toContain("bg-accent");
  });

  it("导航区只剩账户与下载, 「版本」已随多实例模型撤销", () => {
    const markup = renderView("/", true);

    expect(markup).toContain(">账户<");
    expect(markup).toContain(">下载<");
    expect(markup).toContain(">设置<");
    expect(markup).not.toContain(">版本<");
    expect(markup).not.toContain('href="/versions"');
  });
});

describe("Sidebar 材质与圆角", () => {
  it("侧栏只在有壁纸时挂外壳材质, 且可点行不得画出可见方框", () => {
    const markup = renderAt("/");

    // 侧栏是窗口外壳的一部分, 装了壁纸时走最透的那一档; 没装时不挂 ——
    // 那一帧 backdrop-filter 采的是一张纯色, 模糊出来还是同一个颜色, 白烧一次全表面合成。
    // 与 Titlebar 是同一条判据, 两个常驻元素不该对同一个问题给出两个答案。
    expect(markup).not.toContain("surface-shell");
    expect(renderAt("/", true)).toContain("surface-shell");
    // 旧类名已迁走。它只是并行迁移期的安全网, 留在这里会让主控以为还有人在用。
    expect(markup).not.toContain("paper-frost");

    // 行级可点物一律不挂材质。挂上 .surface-control 会给每一行画出一个可见的方框,
    // 四条导航排下来就是四个并列的框; 侧栏本身已经是一块玻璃, 框里套框会把
    // 「一列文字」读成「一列按钮」。这是产品明确否掉的形态, 用例守的就是它不许回来。
    expect(markup).not.toContain("surface-control");

    // 静息无底、悬停才浮一层极淡的墨。悬停底必须还在 ——
    // 一并删掉的话可点行就彻底没有指针反馈了, 那是矫枉过正。
    expect(markup).toContain("hover:bg-ink/6");

    // 当前项的唯一底色线索仍是那道朱红竖规, 全局只有一道。
    expect(markup.split("bg-accent").length - 1).toBe(1);
  });

  it("只有可点的那条游戏行走 .surface-liquid, 且静态首帧不写内联 backdrop-filter", () => {
    const markup = renderView("/", true);

    // 白名单里侧栏只有「可点的游戏行」这一格; 未上线的 Arena 行不许跟着一起发材质 ——
    // 未上线的东西不该比在售的更抢眼。出现两次就是它也被套上了。
    expect(markup.split("surface-liquid").length - 1).toBe(1);

    // 折射透镜写的是内联 backdrop-filter, 优先级压过 .surface-liquid, 也压过 app.css 末尾
    // 那段「减少透明度 / 提高对比度」的降级(它靠 backdrop-filter: none 把玻璃退成实心纸)。
    // 所以静态渲染这一帧必须干净: 问不到 DOM 就没有依据判断玻璃模式与无障碍偏好,
    // 一旦这里冒出内联滤镜, 就说明透镜绕开了全局开关。
    expect(markup).not.toContain("backdrop-filter");
  });

  it("圆角只走令牌类, 弱色阶不得低于玻璃上的正文门槛", () => {
    const markup = renderAt("/");

    expect(markup).toContain("rounded-control");
    // 硬编码圆角是整轮换皮最容易复发的一种回退: 改风格时它不会跟着令牌一起变。
    expect(markup).not.toMatch(/rounded-\[/);

    // 玻璃上 ink/60 在任何一档材质都过不了正文的 4.5 (最坏一格 3.27), 只能做大字与图标。
    // 侧栏的眉标 10px、说明 10px、导航 15px 全是正文尺度, 一律 ink/75 起。
    expect(markup).toContain("text-ink/75");
    expect(markup).not.toContain("text-ink/60");
    expect(markup).not.toContain("text-ink/55");
  });
});
