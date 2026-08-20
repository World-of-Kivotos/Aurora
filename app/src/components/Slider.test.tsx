// 内存滑块这一区里两处真会算错的地方：刻度吸附，与占用条的超额夹逼。
//
// 其余部分不测：滑块本体是原生 <input type="range">，它的键盘、拖拽、可访问性由浏览器负责，
// 测它等于测 Chromium。至于「拖完保存」「开自动锁住滑块」这类跨状态的行为，
// 已经在真 Chromium 里用 CDP 点过一遍（见 scripts/page-smoke.mjs 的同款做法），
// 用 renderToStaticMarkup 复刻一遍只会得到一个测不到真行为的假绿。

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { nearestIndex } from "./Slider";
// ?raw 拿源码本身。走 Vite 的 raw 导入而不是 node:fs：
// 后者要给这个纯浏览器工程装一份 @types/node，为一条接线断言添一个依赖不划算。
import sliderSource from "./Slider.tsx?raw";
import { MemoryBar, gb } from "../pages/Settings";

// 后端 slider_stops(32768) 的真实输出，与 aurora_launch::memory 的三段折线一致。
const STOPS = [
  512, 768, 1024, 1280, 1536, 1792, 2048, 2560, 3072, 3584, 4096, 4608, 5120, 5632, 6144, 7168,
  8192, 9216, 10240, 11264, 12288, 13312, 14336, 16384, 18432, 20480, 22528, 24576, 26624, 28672,
  30720, 32768,
];

describe("nearestIndex", () => {
  it("正好落在格子上时取那一格", () => {
    expect(nearestIndex(STOPS, 512)).toBe(0);
    expect(nearestIndex(STOPS, 8192)).toBe(16);
    expect(nearestIndex(STOPS, 32768)).toBe(STOPS.length - 1);
  });

  it("落在两格之间时取更近的那一格", () => {
    // 6144 与 7168 之间，偏下 -> 6144（下标 14）。
    expect(nearestIndex(STOPS, 6400)).toBe(14);
    // 同一区间偏上 -> 7168（下标 15）。
    expect(nearestIndex(STOPS, 7000)).toBe(15);
  });

  it("正中间时取靠前那一格（严格小于，不接受并列）", () => {
    // 6144 与 7168 的中点是 6656，两侧距离都是 512。
    expect(nearestIndex(STOPS, 6656)).toBe(14);
  });

  it("越界的值夹到两端而不是报错", () => {
    // 手改过 config.json、或从别的启动器迁过来的值可以是任何数，落不到表上是常态。
    expect(nearestIndex(STOPS, 0)).toBe(0);
    expect(nearestIndex(STOPS, 1)).toBe(0);
    expect(nearestIndex(STOPS, 999999)).toBe(STOPS.length - 1);
  });
});

describe("MemoryBar", () => {
  // 32G 机器，别的程序占了 12G，游戏分 8G：可用 20G，剩 12G 空闲。
  const normal = renderToStaticMarkup(
    <MemoryBar totalMb={32768} othersMb={12288} gameMb={8192} />,
  );

  it("三段按各自占整机的比例给宽度", () => {
    expect(normal).toContain("width:37.5%"); // 其他程序 12288/32768
    expect(normal).toContain("width:25%"); // 游戏 8192/32768
  });

  it("图例报的是真实数值，空闲 = 可用 - 游戏", () => {
    expect(normal).toContain("其他程序 12.0 GB");
    expect(normal).toContain("分配给游戏 8.0 GB");
    expect(normal).toContain("空闲 12.0 GB");
    expect(normal).toContain("共 32.0 GB");
  });

  it("没超额时不出警告，也不用危险色", () => {
    expect(normal).not.toContain("已超过此刻可用");
    expect(normal).toContain("bg-accent");
    expect(normal).not.toContain("bg-danger");
  });

  // 同一台机器，游戏要 24G，但此刻只剩 20G 可用。
  const over = renderToStaticMarkup(<MemoryBar totalMb={32768} othersMb={12288} gameMb={24576} />);

  it("超额时转危险色并说清差多少", () => {
    expect(over).toContain("bg-danger");
    expect(over).not.toContain("bg-accent");
    expect(over).toContain("已超过此刻可用的 20.0 GB");
  });

  it("超额时图例仍报玩家真正设的数，条形却夹到可用量", () => {
    // 图例说 24G——那是他设的，不能替他改口。
    expect(over).toContain("分配给游戏 24.0 GB");
    // 条形只画到可用的 20G（20480/32768 = 62.5%），不画一条比容器还长的条把别的段挤没。
    expect(over).toContain("width:62.5%");
    expect(over).toContain("空闲 0.0 GB");
  });

  it("整机内存为 0 这种脏数据不产生 NaN 宽度", () => {
    const zero = renderToStaticMarkup(<MemoryBar totalMb={0} othersMb={0} gameMb={4096} />);
    expect(zero).not.toContain("NaN");
  });
});

describe("Slider 的提交时机", () => {
  // 这一条守的是本组件最容易回退的一处: 把提交接回 React 的 onChange。
  //
  // React 在表单元素上的 onChange 底层绑的是原生 input, 每挪一格触发一次;
  // 而内存滑块的提交会存盘并弹「已保存」, 接错了就是拖一次滑块糊满一屏提示(真发生过)。
  // 静态渲染看不见事件绑定, 所以这里直接读源码断言接线 —— 不优雅, 但它拦得住,
  // 而这个错误一旦回退, 类型、构建、渲染测试全都不会响。
  const source = sliderSource;

  it("提交走原生 change 监听器, 不走 React 的 onChange", () => {
    expect(source).toContain('addEventListener("change"');
    // onCommit 只该出现在类型定义、解构、注释与那个原生监听器里, 不能出现在 JSX 的事件属性上。
    expect(source).not.toMatch(/onChange=\{[^}]*onCommit/);
    expect(source).not.toMatch(/onInput=\{[^}]*onCommit/);
  });

  it("装上的监听器要拆干净, 否则换一次刻度表就多攒一个", () => {
    expect(source).toContain('removeEventListener("change"');
  });

  it("受控输入仍带 onChange(给实时预览), 免得 React 报「有 value 没有 onChange」", () => {
    expect(source).toMatch(/onChange=\{[^}]*onPreview/);
  });
});

describe("gb", () => {
  it("一位小数，单位统一写 GB", () => {
    expect(gb(512)).toBe("0.5 GB");
    expect(gb(6144)).toBe("6.0 GB");
    expect(gb(32768)).toBe("32.0 GB");
  });
});
