// 刻度滑块（受控）。值域不是连续区间，而是一张由调用方给的刻度表，滑块只在下标上移动。
//
// 为什么是刻度表而不是 min/max/step：内存滑块的手感由一条三段折线定义（低内存区 0.25G 一格、
// 常用区 0.5G、再往上 1G / 2G），这条折线的真源在 Rust 侧 aurora_launch::slider_to_mb。
// 前端若改用 min/max/step 就必须把折线复制一份过来，两份实现改一边不改另一边不会有任何报错，
// 只会让手感悄悄漂掉。所以这里只接受一张算好的表，组件自己不懂任何业务换算。
//
// 底层用原生 <input type="range">：键盘（左右/Home/End/PageUp）、无障碍语义、拖拽手感全是免费的，
// 自绘一套 div 只会把这些重新丢一遍。WebView2 是 Chromium，::-webkit-slider-* 伪元素可以完全接管外观，
// 外观复用 app.css 里既有的 .ink-range（壁纸遮罩那根滑条用的同一套），只额外加 .ink-range--filled
// 把「已选到哪」画进轨道——全站只该有一种滑块，另造一套迟早会与它长得不一样。

interface SliderProps {
  /** 刻度表，严格递增。滑块位置是它的下标。 */
  stops: number[];
  /** 当前值。不在表上时吸附到最近的一格。 */
  value: number;
  /** 拖拽结束（change）才回调，拖拽过程中的每一帧不回调——每动一格存一次盘没有意义。 */
  onCommit: (next: number) => void;
  /** 拖拽过程中的实时值，用于让上方读数跟着走。不给则不订阅 input 事件。 */
  onPreview?: (next: number) => void;
  ariaLabel: string;
  /** 读数的显示形式，同时用于 aria-valuetext——屏幕阅读器不该念 "6144"。 */
  format: (mb: number) => string;
  disabled?: boolean;
}

/**
 * 把任意值吸附到刻度表上最近的一格，返回下标。
 *
 * 老配置里的 max_mb 可以是任何数（手改 config.json、或是从别的启动器迁过来的），
 * 落不到格子上是常态而非异常，所以这里取最近而不是报错。
 */
export function nearestIndex(stops: number[], value: number): number {
  let best = 0;
  let bestGap = Number.POSITIVE_INFINITY;
  for (let i = 0; i < stops.length; i++) {
    const gap = Math.abs(stops[i] - value);
    if (gap < bestGap) {
      bestGap = gap;
      best = i;
    }
  }
  return best;
}

export function Slider({
  stops,
  value,
  onCommit,
  onPreview,
  ariaLabel,
  format,
  disabled,
}: SliderProps) {
  // 空表画不出滑块。后端的 slider_stops 保证至少一格，这里只是不让组件在坏数据上崩掉。
  if (stops.length === 0) return null;

  const index = nearestIndex(stops, value);
  const max = stops.length - 1;
  // 已走过的比例交给 CSS 变量：轨道的「已填充」段用 linear-gradient 画，
  // 单独叠一层 absolute 的填充条会在拖拽时与原生 thumb 差一帧。
  const filled = max === 0 ? 1 : index / max;

  return (
    <input
      type="range"
      className="ink-range ink-range--filled w-full"
      min={0}
      max={max}
      step={1}
      value={index}
      disabled={disabled}
      aria-label={ariaLabel}
      // 原生 range 的 aria-valuenow 是下标（0..N），对听觉用户毫无意义，必须用 valuetext 盖掉。
      aria-valuetext={format(stops[index])}
      style={{ ["--aurora-slider-filled" as string]: `${filled * 100}%` }}
      onInput={(event) => onPreview?.(stops[Number(event.currentTarget.value)])}
      onChange={(event) => onCommit(stops[Number(event.currentTarget.value)])}
    />
  );
}
