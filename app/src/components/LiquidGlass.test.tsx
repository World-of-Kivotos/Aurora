// LiquidGlass 的渲染契约。这一层没有业务逻辑, 值得钉死的是四件「坏了也不报错」的事:
// 1) 每个实例的滤镜 id 必须唯一, 否则页面上所有玻璃会共用最后一个滤镜, 表现为尺寸不同的
//    几块玻璃折射强度莫名一致;
// 2) 降级路径必须真的降级 —— 不支持 url() 却照发 url(#id), 元素会连基础模糊一起丢掉,
//    那比不做效果更糟;
// 3) 超尺寸必须自动退回毛玻璃, 这是掉帧的唯一防线;
// 4) 位移图的通道选择器必须与 lib 生成的位移图约定一致(R->x, B->y), 选错会变成单向拉伸。
//
// 用 renderToStaticMarkup 而不是 testing-library: 项目既有做法(见 CrashBanner.test.tsx),
// 且本组件的可断言产物全在标记里 —— 尺寸实测走 ResizeObserver, 本来就不在静态渲染的范围内。

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { LiquidGlass } from "./LiquidGlass";

const SIZE = { width: 400, height: 200 } as const;

function markupOf(node: React.ReactElement): string {
  return renderToStaticMarkup(node);
}

/** 取所有滤镜 id。折射档才有, 毛玻璃档返回空数组。 */
function filterIds(markup: string): string[] {
  return [...markup.matchAll(/<filter id="([^"]+)"/g)].map((match) => match[1]);
}

function countOf(markup: string, needle: string): number {
  return markup.split(needle).length - 1;
}

describe("LiquidGlass 降级路径", () => {
  it("环境探测不通过时(单测的 node 环境)只出毛玻璃, 不出滤镜", () => {
    const markup = markupOf(<LiquidGlass {...SIZE}>内容</LiquidGlass>);

    expect(markup).toContain("backdrop-filter:blur(12px) saturate(160%)");
    expect(markup).not.toContain("<filter");
    expect(markup).not.toContain("url(#");
    expect(markup).toContain("内容");
  });

  it("调用方点名要毛玻璃时, 环境与尺寸都达标也不折射", () => {
    const markup = markupOf(<LiquidGlass {...SIZE} supportsUrlFilter mode="frost" />);

    expect(markup).toContain("backdrop-filter:blur(12px) saturate(160%)");
    expect(filterIds(markup)).toHaveLength(0);
  });

  it("尺寸未知时先出毛玻璃 —— 位移图没有尺寸就无从生成", () => {
    const markup = markupOf(<LiquidGlass supportsUrlFilter />);

    expect(filterIds(markup)).toHaveLength(0);
    expect(markup).toContain("backdrop-filter:blur(12px)");
  });

  it("超过宽度上限时自动退回毛玻璃(掉帧防线)", () => {
    const wide = markupOf(<LiquidGlass width={900} height={200} supportsUrlFilter />);
    const atLimit = markupOf(<LiquidGlass width={800} height={200} supportsUrlFilter />);

    expect(filterIds(wide)).toHaveLength(0);
    expect(filterIds(atLimit)).toHaveLength(1);
  });

  it("调用方可以把宽度上限调得更严", () => {
    const markup = markupOf(
      <LiquidGlass {...SIZE} supportsUrlFilter maxRefractionWidth={320} />,
    );

    expect(filterIds(markup)).toHaveLength(0);
  });

  it("毛玻璃档尊重调用方给的更大模糊", () => {
    const markup = markupOf(<LiquidGlass {...SIZE} blur={28} />);

    expect(markup).toContain("backdrop-filter:blur(28px) saturate(160%)");
  });
});

describe("LiquidGlass 折射档", () => {
  const markup = markupOf(
    <LiquidGlass {...SIZE} supportsUrlFilter className="rounded-panel">
      折射内容
    </LiquidGlass>,
  );
  const id = filterIds(markup)[0];

  it("元素上只有一个 url(), 指向本实例的滤镜", () => {
    expect(id).toMatch(/^aurora-lens-/);
    expect(markup).toContain(`backdrop-filter:url(#${id})`);
    expect(markup).not.toContain("backdrop-filter:blur(");
    // 不带前缀的那一条是唯一的一条: 多挂一条 -webkit- 孪生属性会在 style 覆盖时留下旧值。
    expect(markup).not.toContain("-webkit-backdrop-filter");
  });

  it("滤镜坐标系与元素 border box 一比一对齐", () => {
    expect(markup).toContain('filterUnits="userSpaceOnUse"');
    expect(markup).toContain('primitiveUnits="userSpaceOnUse"');
    expect(markup).toContain('color-interpolation-filters="sRGB"');
    expect(markup).toContain('x="0" y="0" width="400" height="200"');
  });

  it("位移图按实际尺寸生成, 以 data URI 喂给 feImage", () => {
    expect(markup).toContain('<feImage href="data:image/svg+xml;charset=utf-8,');
    expect(markup).toContain('preserveAspectRatio="none"');
    expect(markup).toContain("%3Csvg%20xmlns");
  });

  it("先模糊、再补饱和、最后位移", () => {
    expect(markup).toContain('<feGaussianBlur in="SourceGraphic" stdDeviation="4" result="softened"');
    expect(markup).toContain('in="softened" type="saturate" values="1.6" result="tinted"');
    expect(markup).toContain('<feDisplacementMap in="tinted"');
  });

  it("通道选择器与位移图约定一致: R 承载 x, B 承载 y", () => {
    expect(countOf(markup, 'xChannelSelector="R" yChannelSelector="B"')).toBe(3);
    expect(markup).not.toContain('yChannelSelector="G"');
  });

  it("色散跑三遍位移, 三个强度按红<绿<蓝排, 再各留一个通道 screen 回去", () => {
    expect(countOf(markup, "<feDisplacementMap")).toBe(3);
    expect(markup).toContain('scale="23.92"');
    expect(markup).toContain('scale="26"');
    expect(markup).toContain('scale="28.08"');
    expect(countOf(markup, '<feBlend')).toBe(2);
    expect(countOf(markup, 'mode="screen"')).toBe(2);
  });

  it("承载滤镜的 svg 不参与布局也不参与无障碍树, 且绝不是 display:none", () => {
    expect(markup).toContain('aria-hidden="true"');
    expect(markup).toContain("position:absolute;width:0;height:0;overflow:hidden");
    expect(markup).not.toContain("display:none");
  });

  it("className 原样透传, 圆角由调用方的令牌类决定", () => {
    expect(markup).toContain('class="rounded-panel"');
    expect(markup).toContain("折射内容");
  });
});

describe("LiquidGlass 实例隔离与开关", () => {
  it("同一棵树里的两个实例各有各的滤镜 id, 互不串", () => {
    const markup = markupOf(
      <div>
        <LiquidGlass {...SIZE} supportsUrlFilter />
        <LiquidGlass width={300} height={120} supportsUrlFilter />
      </div>,
    );
    const ids = filterIds(markup);

    expect(ids).toHaveLength(2);
    expect(new Set(ids).size).toBe(2);
    expect(markup).toContain(`backdrop-filter:url(#${ids[0]})`);
    expect(markup).toContain(`backdrop-filter:url(#${ids[1]})`);
  });

  it("关掉色散只跑一遍位移, 不再有通道拆分与合成", () => {
    const markup = markupOf(<LiquidGlass {...SIZE} supportsUrlFilter dispersion={0} />);

    expect(countOf(markup, "<feDisplacementMap")).toBe(1);
    expect(markup).toContain('scale="26"');
    expect(markup).not.toContain("<feBlend");
    expect(markup).not.toContain('type="matrix"');
  });

  it("默认带受光边与描边, sheen 关掉后把 box-shadow 让给调用方", () => {
    const on = markupOf(<LiquidGlass {...SIZE} />);
    const off = markupOf(<LiquidGlass {...SIZE} sheen={false} />);

    expect(on).toContain("box-shadow:var(--glass-sheen), var(--glass-rim)");
    expect(off).not.toContain("box-shadow");
  });

  it("调用方给的 style 压过组件默认值", () => {
    const markup = markupOf(<LiquidGlass {...SIZE} style={{ backdropFilter: "none" }} />);

    expect(markup).toContain("backdrop-filter:none");
    expect(markup).not.toContain("backdrop-filter:blur(12px)");
  });
});
