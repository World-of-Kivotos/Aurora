/*
 * 液态玻璃的 React 外壳。真正的算法在 ../lib/liquid-glass, 这里只负责三件事:
 *   1. 拿到元素的像素尺寸(位移图必须按实际大小生成, 拉伸的位移图会让边缘厚度失真);
 *   2. 给每个实例分配一个独一无二的滤镜 id;
 *   3. 按环境与尺寸定档, 该降级就降级。
 *
 * 刻意不做的两件事(留给调用方, 别在这里加):
 *   - 不设背景色。玻璃的纸色配比是 app.css 里那张对比度预算表的事, 调用方用材质类或
 *     bg-* 工具类自己给; 这个组件只出「透镜」, 不出「纸」。
 *   - 不设圆角。圆角一律走 rounded-panel / rounded-control / rounded-chip 三个令牌类,
 *     由调用方写在 className 上; backdrop-filter 天然被 border-radius 裁切, 圆角因此
 *     自动生效, 组件不必知道半径是多少。
 */

import { useEffect, useId, useMemo, useRef, useState, type HTMLAttributes, type ReactNode } from "react";
import {
  DEFAULT_PARAMS,
  KEEP_CHANNEL,
  MAX_REFRACTION_WIDTH,
  backdropFilterValue,
  buildDisplacementMapDataUri,
  dispersionScales,
  needsDispersion,
  probeUrlFilterSupport,
  resolveGlassMode,
  sanitizeFilterId,
  saturationRatio,
  type GlassMode,
  type GlassModeRequest,
} from "../lib/liquid-glass";

export interface LiquidGlassProps extends Omit<HTMLAttributes<HTMLDivElement>, "children"> {
  children?: ReactNode;
  /** 位移强度(px), 默认 26。 */
  strength?: number;
  /** 折射边厚度(px), 默认 22。 */
  bevel?: number;
  /** 色散比例(0..1), 默认 0.08; 给 0 可省掉两遍位移采样。 */
  dispersion?: number;
  /** 背景模糊(px), 默认 4; 降级到毛玻璃时会被抬到 FROST_MIN_BLUR。 */
  blur?: number;
  /** 饱和度(%), 默认 160。 */
  saturation?: number;
  /** 顶缘受光亮边 + 发丝描边, 默认开。关掉后 className 里的 shadow-* 才有机会生效。 */
  sheen?: boolean;
  /** "frost" 表示调用方主动要毛玻璃(全局 frost 模式 / 无障碍偏好), 默认 "auto"。 */
  mode?: GlassModeRequest;
  /** 显式宽度(px)。给了就不装 ResizeObserver —— 尺寸本来就固定时没必要每帧去量。 */
  width?: number;
  /** 显式高度(px), 与 width 同理; 两者可以只给一个, 另一个仍走实测。 */
  height?: number;
  /** 折射档的宽度上限(px), 默认 800。想更保守就调小。 */
  maxRefractionWidth?: number;
  /**
   * 覆盖环境探测结果。留空时组件自己探(结果全局缓存)。
   * 供上层把探测提到一处(设置面板/上下文)统一下发, 免得每个实例各探一次。
   */
  supportsUrlFilter?: boolean;
}

export function LiquidGlass({
  children,
  className,
  style,
  strength = DEFAULT_PARAMS.strength,
  bevel = DEFAULT_PARAMS.bevel,
  dispersion = DEFAULT_PARAMS.dispersion,
  blur = DEFAULT_PARAMS.blur,
  saturation = DEFAULT_PARAMS.saturation,
  sheen = true,
  mode: requestedMode = "auto",
  width,
  height,
  maxRefractionWidth = MAX_REFRACTION_WIDTH,
  supportsUrlFilter,
  ...rest
}: LiquidGlassProps) {
  const filterId = sanitizeFilterId(useId());
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [measured, setMeasured] = useState<{ width: number; height: number } | null>(null);

  const sizeIsFixed = width !== undefined && height !== undefined;

  useEffect(() => {
    if (sizeIsFixed) return;
    const host = hostRef.current;
    // ResizeObserver 在测试用的 node 环境里不存在; 量不到就一直是毛玻璃, 这是安全的那一侧。
    if (host === null || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver((entries) => {
      if (entries.length === 0) return;
      const entry = entries[0];
      // 取 border box 而不是 content box: backdrop-filter 的作用范围是 border box,
      // 用内容盒生成的位移图会比实际玻璃小一圈, 表现为边缘折射整体内缩。
      const box = entry.borderBoxSize.length > 0 ? entry.borderBoxSize[0] : null;
      const nextWidth = Math.round(box === null ? entry.contentRect.width : box.inlineSize);
      const nextHeight = Math.round(box === null ? entry.contentRect.height : box.blockSize);
      // 取整后再比对: 亚像素抖动会让位移图字符串每帧都变, feImage 每帧重新解码一张图。
      setMeasured((prev) =>
        prev !== null && prev.width === nextWidth && prev.height === nextHeight
          ? prev
          : { width: nextWidth, height: nextHeight },
      );
    });
    observer.observe(host);
    return () => observer.disconnect();
  }, [sizeIsFixed]);

  const resolvedWidth = width === undefined ? (measured === null ? null : measured.width) : width;
  const resolvedHeight = height === undefined ? (measured === null ? null : measured.height) : height;

  const supported = supportsUrlFilter === undefined ? probeUrlFilterSupport() : supportsUrlFilter;
  const resolved = resolveGlassMode({
    requested: requestedMode,
    supportsUrlFilter: supported,
    width: resolvedWidth,
    height: resolvedHeight,
    maxRefractionWidth,
  });

  // 位移图字符串不便宜(一张 data URI 要走一次 encodeURIComponent), 尺寸不变就不重算。
  const lens = useMemo(() => {
    if (resolved !== "refract" || resolvedWidth === null || resolvedHeight === null) return null;
    return {
      width: resolvedWidth,
      height: resolvedHeight,
      mapUri: buildDisplacementMapDataUri({
        width: resolvedWidth,
        height: resolvedHeight,
        bevel,
      }),
      scales: dispersionScales(strength, dispersion),
      dispersed: needsDispersion(strength, dispersion),
    };
  }, [resolved, resolvedWidth, resolvedHeight, bevel, strength, dispersion]);

  const activeMode: GlassMode = lens === null ? "frost" : "refract";
  const filterValue = backdropFilterValue(activeMode, filterId, {
    strength,
    bevel,
    dispersion,
    blur,
    saturation,
  });

  const glassStyle = {
    // 只写不带前缀的那一条: Chromium 76+ 与 Safari 18+ 都已支持无前缀形态, 而多挂一条
    // -webkit- 孪生属性会在调用方用 style 覆盖 backdropFilter 时留下一条对不上的旧值,
    // 表现为「明明覆盖了却还有一层模糊」。
    backdropFilter: filterValue,
    ...(sheen ? { boxShadow: "var(--glass-sheen), var(--glass-rim)" } : {}),
    // 调用方的 style 放最后: 组件给的是默认值, 不是不可推翻的结论。
    ...style,
  };

  return (
    <div ref={hostRef} className={className} style={glassStyle} {...rest}>
      {lens === null ? null : (
        <svg
          aria-hidden="true"
          focusable="false"
          width="0"
          height="0"
          // 零尺寸 + absolute 让它彻底不参与布局。绝不能用 display:none —— 那样滤镜
          // 定义会被当成不渲染的内容, url(#id) 直接解析失败, 玻璃变成一块空洞。
          style={{ position: "absolute", width: 0, height: 0, overflow: "hidden" }}
        >
          <filter
            id={filterId}
            // 两个 units 都用 userSpaceOnUse, 配合下面按像素写死的区域, 让滤镜坐标系与
            // 元素的 border box 一比一对齐; 用默认的比例单位会让位移强度随元素尺寸漂移。
            filterUnits="userSpaceOnUse"
            primitiveUnits="userSpaceOnUse"
            // 不写 sRGB 的话 SVG 默认在线性空间做运算, 模糊与饱和的结果会与 CSS 同名
            // 滤镜对不上, 同一块界面上两种玻璃会呈现两种色调。
            colorInterpolationFilters="sRGB"
            x={0}
            y={0}
            width={lens.width}
            height={lens.height}
          >
            <feImage
              href={lens.mapUri}
              x={0}
              y={0}
              width={lens.width}
              height={lens.height}
              preserveAspectRatio="none"
              result="lensMap"
            />
            <feGaussianBlur in="SourceGraphic" stdDeviation={blur} result="softened" />
            <feColorMatrix
              in="softened"
              type="saturate"
              values={String(saturationRatio(saturation))}
              result="tinted"
            />
            {lens.dispersed ? (
              <>
                <feDisplacementMap
                  in="tinted"
                  in2="lensMap"
                  scale={lens.scales.r}
                  xChannelSelector="R"
                  yChannelSelector="B"
                  result="shiftR"
                />
                <feColorMatrix in="shiftR" type="matrix" values={KEEP_CHANNEL.r} result="onlyR" />
                <feDisplacementMap
                  in="tinted"
                  in2="lensMap"
                  scale={lens.scales.g}
                  xChannelSelector="R"
                  yChannelSelector="B"
                  result="shiftG"
                />
                <feColorMatrix in="shiftG" type="matrix" values={KEEP_CHANNEL.g} result="onlyG" />
                <feDisplacementMap
                  in="tinted"
                  in2="lensMap"
                  scale={lens.scales.b}
                  xChannelSelector="R"
                  yChannelSelector="B"
                  result="shiftB"
                />
                <feColorMatrix in="shiftB" type="matrix" values={KEEP_CHANNEL.b} result="onlyB" />
                {/* 三个单通道图各自只有一个通道非零, screen 在这里等价于按通道相加。 */}
                <feBlend in="onlyR" in2="onlyG" mode="screen" result="lensRG" />
                <feBlend in="lensRG" in2="onlyB" mode="screen" />
              </>
            ) : (
              <feDisplacementMap
                in="tinted"
                in2="lensMap"
                scale={strength}
                xChannelSelector="R"
                yChannelSelector="B"
              />
            )}
          </filter>
        </svg>
      )}
      {children}
    </div>
  );
}
