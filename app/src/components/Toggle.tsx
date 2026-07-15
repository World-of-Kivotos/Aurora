// 自定义开关控件（受控），替代系统默认 checkbox。
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
        "relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        "disabled:pointer-events-none disabled:opacity-40",
        checked ? "border-ink bg-ink" : "border-ink/20 bg-paper-sink",
      ].join(" ")}
    >
      <span
        className={[
          "inline-block h-[18px] w-[18px] rounded-full transition-transform motion-reduce:transition-none",
          checked ? "translate-x-[22px] bg-paper-on" : "translate-x-[3px] bg-ink/55",
        ].join(" ")}
      />
    </button>
  );
}
