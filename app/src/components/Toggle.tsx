// 自定义开关控件（受控），替代系统默认 checkbox（WebView2 的原生外观是系统蓝，与纸墨版面打架）。
// role="switch" + aria-checked，键盘可聚焦/可激活（button 原生支持 Enter/Space），焦点可见；
// 滑块位移走 transition，尊重 prefers-reduced-motion（motion-reduce 关闭过渡）。

interface ToggleProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  ariaLabel: string;
  id?: string;
  disabled?: boolean;
}

export function Toggle({ checked, onChange, ariaLabel, id, disabled }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      id={id}
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={[
        "relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full transition-colors",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        "disabled:pointer-events-none disabled:opacity-40",
        // 开=满墨实底（唯一一处不留玻璃的地方：开关的「开」必须一眼看死，半透明会让它在浅色照片上变含糊）；
        // 关=下沉轨，与进度轨同材质。两态都不带 border，描边由材质的 inset 阴影给，盒子尺寸不受影响。
        checked ? "bg-ink" : "surface-sunken",
      ].join(" ")}
    >
      {/*
        关态钮从 ink/55 提到 ink/60：轨道本身已是 8% 墨洗，钮与轨的差要撑到非文字对比度的 3:1，
        ink/60 是色阶表里唯一既在档、又刚好过线（最差 3.27）的一档。
        位移取 3 / 23：去掉 border 之后可用宽度是整 44px，两端各留 3px 才对称。
      */}
      <span
        className={[
          "inline-block h-[18px] w-[18px] rounded-full transition-transform motion-reduce:transition-none",
          checked ? "translate-x-[23px] bg-paper-on" : "translate-x-[3px] bg-ink/60",
        ].join(" ")}
      />
    </button>
  );
}
