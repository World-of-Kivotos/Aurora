// 极简侧栏：品牌标 + 四项导航（主页/账户/下载/设置）。当前项一道朱红竖规 + 加粗，无编号、无页脚装饰。
// 竖规用 framer-motion 的 layoutId 在切换时平滑滑动。路由用 NavLink，与 App.tsx 一一对应。

import { NavLink } from "react-router-dom";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { SparkleIcon } from "./icons";

interface NavDef {
  to: string;
  label: string;
  end?: boolean;
}

const TOP: NavDef[] = [
  { to: "/", label: "主页", end: true },
  { to: "/account", label: "账户" },
  { to: "/versions", label: "版本" },
  { to: "/download", label: "下载" },
];

const BOTTOM: NavDef[] = [{ to: "/settings", label: "设置" }];

function NavRow({ to, label, end }: NavDef) {
  return (
    <NavLink
      to={to}
      end={end}
      className="group relative flex items-center rounded-[3px] py-[10px] pr-3 pl-[16px] transition-colors hover:bg-ink/4 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
    >
      {({ isActive }) => (
        <>
          {isActive && (
            <motion.span
              layoutId="nav-rule"
              transition={springs.soft}
              className="absolute top-[8px] bottom-[8px] left-0 w-[2px] bg-accent"
            />
          )}
          <span
            className={[
              "text-[15px] tracking-[0.02em] transition-colors",
              isActive ? "font-extrabold text-ink" : "font-semibold text-ink/55 group-hover:text-ink",
            ].join(" ")}
          >
            {label}
          </span>
        </>
      )}
    </NavLink>
  );
}

export function Sidebar() {
  return (
    <nav
      aria-label="主导航"
      className="flex w-[196px] shrink-0 flex-col border-r border-ink/10 px-5 pt-6 pb-5"
    >
      <div className="mb-9 flex items-center gap-2.5 pl-[6px]">
        <SparkleIcon size={22} className="text-ink" />
        <span className="text-[21px] leading-none font-extrabold tracking-[-0.02em]">Aurora</span>
      </div>

      <ul className="flex flex-col gap-0.5">
        {TOP.map((it) => (
          <li key={it.to}>
            <NavRow {...it} />
          </li>
        ))}
      </ul>

      <div className="mt-auto">
        {BOTTOM.map((it) => (
          <NavRow key={it.to} {...it} />
        ))}
      </div>
    </nav>
  );
}
