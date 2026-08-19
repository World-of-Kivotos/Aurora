// 液态玻璃的纯逻辑契约。这一层的每个分支坏掉都不会抛异常, 只会安静地画错一块玻璃:
// 位移图的 stop 倒序会变成一圈锯齿, 通道选错会变成只往一个方向拉伸, 尺寸门槛失效会掉帧,
// id 消毒失效会让页面上所有玻璃共用同一个滤镜。所以这里断言的是具体数值与具体分支,
// 不是「函数有返回值」。

import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_PARAMS,
  FROST_MIN_BLUR,
  KEEP_CHANNEL,
  MAX_REFRACTION_WIDTH,
  backdropFilterValue,
  bevelStops,
  buildDisplacementMapDataUri,
  buildDisplacementMapSvg,
  detectUrlFilterSupport,
  dispersionScales,
  needsDispersion,
  resolveGlassMode,
  sanitizeFilterId,
  saturationRatio,
  type GlassModeInput,
} from "./liquid-glass";

const DATA_URI_PREFIX = "data:image/svg+xml;charset=utf-8,";

/** 按 feColorMatrix type="matrix" 的定义把 20 个数应用到一个颜色上, 用来验矩阵语义而不是字符串。 */
function applyColorMatrix(values: string, color: [number, number, number, number]): number[] {
  const m = values.trim().split(/\s+/).map(Number);
  expect(m).toHaveLength(20);
  const [r, g, b, a] = color;
  return [0, 1, 2, 3].map((row) => {
    const o = row * 5;
    return m[o] * r + m[o + 1] * g + m[o + 2] * b + m[o + 3] * a + m[o + 4];
  });
}

describe("bevelStops", () => {
  it("把边厚换算成两侧的渐变停靠点", () => {
    expect(bevelStops(200, 20)).toEqual({ near: 0.1, far: 0.9 });
    expect(bevelStops(400, 20)).toEqual({ near: 0.05, far: 0.95 });
  });

  it("边厚为 0 时整块都是中性区, 两个停靠点落在两端", () => {
    expect(bevelStops(300, 0)).toEqual({ near: 0, far: 1 });
    expect(bevelStops(300, -10)).toEqual({ near: 0, far: 1 });
  });

  it("边厚超过半边长时夹到 0.5, 绝不产出倒序的 offset", () => {
    expect(bevelStops(200, 100)).toEqual({ near: 0.5, far: 0.5 });
    expect(bevelStops(200, 500)).toEqual({ near: 0.5, far: 0.5 });
  });

  it("尺寸非法时同样退到中线, 不产出 NaN 或 Infinity", () => {
    expect(bevelStops(0, 20)).toEqual({ near: 0.5, far: 0.5 });
    expect(bevelStops(-5, 20)).toEqual({ near: 0.5, far: 0.5 });
    expect(bevelStops(Number.NaN, 20)).toEqual({ near: 0.5, far: 0.5 });
  });

  it("除不尽时两端各自四舍五入到万分位", () => {
    expect(bevelStops(3, 1)).toEqual({ near: 0.3333, far: 0.6667 });
  });
});

describe("buildDisplacementMapSvg", () => {
  const svg = buildDisplacementMapSvg({ width: 400, height: 200, bevel: 20 });

  it("画布尺寸与 viewBox 跟随几何", () => {
    expect(svg).toContain('width="400" height="200" viewBox="0 0 400 200"');
  });

  it("R 通道沿 x 轴、B 通道沿 y 轴, 中间是 128 的中性区", () => {
    expect(svg).toContain('<linearGradient id="x" x1="0" y1="0" x2="1" y2="0">');
    expect(svg).toContain('<stop offset="0" stop-color="rgb(0,0,0)"/>');
    expect(svg).toContain('<stop offset="0.05" stop-color="rgb(128,0,0)"/>');
    expect(svg).toContain('<stop offset="0.95" stop-color="rgb(128,0,0)"/>');
    expect(svg).toContain('<stop offset="1" stop-color="rgb(255,0,0)"/>');

    expect(svg).toContain('<linearGradient id="y" x1="0" y1="0" x2="0" y2="1">');
    expect(svg).toContain('<stop offset="0.1" stop-color="rgb(0,0,128)"/>');
    expect(svg).toContain('<stop offset="0.9" stop-color="rgb(0,0,128)"/>');
    expect(svg).toContain('<stop offset="1" stop-color="rgb(0,0,255)"/>');
  });

  it("三层结构: 黑底 + x 渐变 + screen 上去的 y 渐变", () => {
    expect(svg.split("<rect ")).toHaveLength(4);
    expect(svg).toContain('<rect width="400" height="200" fill="rgb(0,0,0)"/>');
    expect(svg).toContain('<rect width="400" height="200" fill="url(#x)"/>');
    // 只有 y 那层混合。两层都 screen 会把黑底也算进去, 结果一样; 但三层里若有两层带
    // mix-blend-mode, 说明有人把打底的黑矩形也改成了混合层, 那时通道会串。
    expect(svg.split("mix-blend-mode:screen")).toHaveLength(2);
    expect(svg).toContain('fill="url(#y)" style="mix-blend-mode:screen"');
  });

  it("尺寸取整, 亚像素不会生成两串不同的位移图", () => {
    const a = buildDisplacementMapSvg({ width: 400.4, height: 200.2, bevel: 20 });
    const b = buildDisplacementMapSvg({ width: 399.6, height: 199.8, bevel: 20 });
    expect(a).toBe(svg);
    expect(b).toBe(svg);
  });
});

describe("buildDisplacementMapDataUri", () => {
  const geometry = { width: 320, height: 120, bevel: 16 };
  const uri = buildDisplacementMapDataUri(geometry);

  it("带正确的 data 前缀, 且能原样还原成 SVG", () => {
    expect(uri.startsWith(DATA_URI_PREFIX)).toBe(true);
    expect(decodeURIComponent(uri.slice(DATA_URI_PREFIX.length))).toBe(
      buildDisplacementMapSvg(geometry),
    );
  });

  it("井号必须被转义, 否则整张图会在 url(#x) 处被当成片段截断", () => {
    expect(uri).not.toContain("#");
    expect(uri).toContain("%23x");
    expect(uri).toContain("%23y");
  });
});

describe("dispersionScales", () => {
  it("蓝端偏折最大、红端最小, 绿端就是基准强度", () => {
    // 精确相等而不是 toBeCloseTo: 这三个数会原样写进 SVG 属性, 浮点尾巴必须已经被消掉。
    expect(dispersionScales(30, 0.1)).toEqual({ r: 27, g: 30, b: 33 });
    expect(dispersionScales(26, 0.08)).toEqual({ r: 23.92, g: 26, b: 28.08 });
  });

  it("色散比例夹在 0..1, 越界不会把红端推成负位移", () => {
    expect(dispersionScales(30, 2)).toEqual({ r: 0, g: 30, b: 60 });
    expect(dispersionScales(30, -1)).toEqual({ r: 30, g: 30, b: 30 });
    expect(dispersionScales(30, Number.NaN)).toEqual({ r: 30, g: 30, b: 30 });
  });
});

describe("needsDispersion", () => {
  it("默认参数要跑三遍位移", () => {
    expect(needsDispersion(DEFAULT_PARAMS.strength, DEFAULT_PARAMS.dispersion)).toBe(true);
  });

  it("色散为 0 或强度为 0 时退回单次位移", () => {
    expect(needsDispersion(26, 0)).toBe(false);
    expect(needsDispersion(26, -1)).toBe(false);
    expect(needsDispersion(0, 0.08)).toBe(false);
  });
});

describe("KEEP_CHANNEL", () => {
  it("每个矩阵只留一个通道并保住 alpha", () => {
    const color: [number, number, number, number] = [0.9, 0.6, 0.3, 1];
    expect(applyColorMatrix(KEEP_CHANNEL.r, color)).toEqual([0.9, 0, 0, 1]);
    expect(applyColorMatrix(KEEP_CHANNEL.g, color)).toEqual([0, 0.6, 0, 1]);
    expect(applyColorMatrix(KEEP_CHANNEL.b, color)).toEqual([0, 0, 0.3, 1]);
  });

  it("三份加回去正好还原原色 —— 这是 screen 合成成立的前提", () => {
    const color: [number, number, number, number] = [0.9, 0.6, 0.3, 1];
    const sum = [KEEP_CHANNEL.r, KEEP_CHANNEL.g, KEEP_CHANNEL.b]
      .map((values) => applyColorMatrix(values, color))
      .reduce((acc, cur) => acc.map((value, index) => value + cur[index]));
    expect(sum.slice(0, 3)).toEqual([0.9, 0.6, 0.3]);
  });
});

describe("saturationRatio", () => {
  it("百分比换算成倍率并夹在 0..4", () => {
    expect(saturationRatio(160)).toBeCloseTo(1.6, 10);
    expect(saturationRatio(100)).toBe(1);
    expect(saturationRatio(0)).toBe(0);
    expect(saturationRatio(500)).toBe(4);
    expect(saturationRatio(-10)).toBe(0);
    expect(saturationRatio(Number.NaN)).toBe(0);
  });
});

describe("detectUrlFilterSupport", () => {
  it("两道判据都过才算支持", () => {
    expect(detectUrlFilterSupport({ cssSupports: () => true, hasUserAgentData: true })).toBe(true);
  });

  it("CSS 认这条声明但不是 Chromium 时判为不支持(解析得过、渲染不出来)", () => {
    expect(detectUrlFilterSupport({ cssSupports: () => true, hasUserAgentData: false })).toBe(false);
  });

  it("是 Chromium 但 CSS 不认时同样判为不支持", () => {
    expect(detectUrlFilterSupport({ cssSupports: () => false, hasUserAgentData: true })).toBe(false);
  });

  it("拿不到 CSS.supports 的环境(node)一律判为不支持", () => {
    expect(detectUrlFilterSupport({})).toBe(false);
    expect(detectUrlFilterSupport({ hasUserAgentData: true })).toBe(false);
  });

  it("问的是 backdrop-filter 的 url() 形态, 不是别的属性", () => {
    const cssSupports = vi.fn<(property: string, value: string) => boolean>(() => true);
    detectUrlFilterSupport({ cssSupports, hasUserAgentData: true });
    expect(cssSupports).toHaveBeenCalledOnce();
    expect(cssSupports.mock.calls[0][0]).toBe("backdrop-filter");
    expect(cssSupports.mock.calls[0][1]).toMatch(/^url\(#.+\)$/);
  });
});

describe("resolveGlassMode", () => {
  const base: GlassModeInput = {
    requested: "auto",
    supportsUrlFilter: true,
    width: 400,
    height: 200,
    maxRefractionWidth: MAX_REFRACTION_WIDTH,
  };

  it("环境支持且尺寸达标时才折射", () => {
    expect(resolveGlassMode(base)).toBe("refract");
  });

  it("调用方点名要毛玻璃时不再考虑其它条件", () => {
    expect(resolveGlassMode({ ...base, requested: "frost" })).toBe("frost");
  });

  it("环境不支持 url() 滤镜时降级", () => {
    expect(resolveGlassMode({ ...base, supportsUrlFilter: false })).toBe("frost");
  });

  it("尺寸还没量到时降级 —— 位移图没有尺寸就无从生成", () => {
    expect(resolveGlassMode({ ...base, width: null })).toBe("frost");
    expect(resolveGlassMode({ ...base, height: null })).toBe("frost");
  });

  it("量到 0 或非法尺寸时降级", () => {
    expect(resolveGlassMode({ ...base, width: 0 })).toBe("frost");
    expect(resolveGlassMode({ ...base, height: 0 })).toBe("frost");
    expect(resolveGlassMode({ ...base, width: -10 })).toBe("frost");
    expect(resolveGlassMode({ ...base, width: Number.NaN })).toBe("frost");
  });

  it("宽度上限含等号: 800 仍折射, 801 掉回毛玻璃", () => {
    expect(resolveGlassMode({ ...base, width: 800 })).toBe("refract");
    expect(resolveGlassMode({ ...base, width: 801 })).toBe("frost");
  });

  it("调用方可以把上限调得更严", () => {
    expect(resolveGlassMode({ ...base, maxRefractionWidth: 320 })).toBe("frost");
    expect(resolveGlassMode({ ...base, width: 320, maxRefractionWidth: 320 })).toBe("refract");
  });

  it("高度不参与门槛判定(没有实测数据支撑的门槛不编)", () => {
    expect(resolveGlassMode({ ...base, height: 4000 })).toBe("refract");
  });
});

describe("sanitizeFilterId", () => {
  it("React 19.2 的 id 形态原样可用, 只加前缀", () => {
    expect(sanitizeFilterId("_R_1_")).toBe("aurora-lens-_R_1_");
  });

  it("剔掉历史版本 useId 里的非法 XML 名字符", () => {
    expect(sanitizeFilterId("«r0»")).toBe("aurora-lens-r0");
    expect(sanitizeFilterId(":r1:")).toBe("aurora-lens-r1");
  });

  it("保留大小写 —— 一起小写化会把两个不同实例撞成同一个 id", () => {
    expect(sanitizeFilterId("«R0»")).not.toBe(sanitizeFilterId("«r0»"));
  });

  it("不同实例消毒后仍然彼此不同", () => {
    const ids = ["_R_1_", "_R_2_", "_R_3_"].map(sanitizeFilterId);
    expect(new Set(ids).size).toBe(3);
  });

  it("字符被剔光时给固定兜底, 不产出空 id", () => {
    expect(sanitizeFilterId("《》")).toBe("aurora-lens-anon");
  });
});

describe("backdropFilterValue", () => {
  it("折射档只留一个 url(), 模糊与饱和都在滤镜内部", () => {
    expect(backdropFilterValue("refract", "aurora-lens-_R_1_", DEFAULT_PARAMS)).toBe(
      "url(#aurora-lens-_R_1_)",
    );
  });

  it("毛玻璃档把模糊抬到下限 —— 位移没了就得靠模糊扛玻璃感", () => {
    expect(backdropFilterValue("frost", "unused", DEFAULT_PARAMS)).toBe(
      `blur(${FROST_MIN_BLUR}px) saturate(160%)`,
    );
  });

  it("调用方给的模糊高于下限时原样用", () => {
    expect(backdropFilterValue("frost", "unused", { ...DEFAULT_PARAMS, blur: 28 })).toBe(
      "blur(28px) saturate(160%)",
    );
  });

  it("饱和度越界时夹紧, 不会吐出非法的 CSS", () => {
    expect(backdropFilterValue("frost", "unused", { ...DEFAULT_PARAMS, saturation: 900 })).toBe(
      `blur(${FROST_MIN_BLUR}px) saturate(400%)`,
    );
    expect(
      backdropFilterValue("frost", "unused", { ...DEFAULT_PARAMS, saturation: Number.NaN }),
    ).toBe(`blur(${FROST_MIN_BLUR}px) saturate(0%)`);
  });
});
