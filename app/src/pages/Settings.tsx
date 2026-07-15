// 设置页：示范自定义 Toggle 开关（替代系统默认控件）。
// 本轮为本地 UI 状态；写入 aurora-core 配置（getConfig/save）将由后续 agent 接入，不在此伪造持久化。

import { useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Toggle } from "../components/Toggle";
import { pageItem } from "../lib/motion";

interface SettingRow {
  id: string;
  title: string;
  desc: string;
}

const ROWS: SettingRow[] = [
  { id: "auto-java", title: "自动下载 Java", desc: "缺失运行时会自动获取匹配版本的 JRE" },
  { id: "isolation", title: "版本隔离", desc: "为每个版本使用独立的存档与配置目录" },
  { id: "mirror", title: "下载镜像优先", desc: "优先走国内镜像源，失败回退官方源" },
  { id: "snapshot", title: "显示快照版本", desc: "在版本列表中包含开发中的快照" },
];

export function Settings() {
  const [state, setState] = useState<Record<string, boolean>>({
    "auto-java": true,
    isolation: true,
    mirror: true,
    snapshot: false,
  });

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title="设置" subtitle="下载源、内存与目录" />
      </motion.div>

      <motion.div variants={pageItem} className="flex flex-col">
        {ROWS.map((r) => (
          <div
            key={r.id}
            className="flex items-center justify-between gap-6 border-b border-ink/9 py-[18px] last:border-b-0"
          >
            <div className="min-w-0">
              <div className="text-[15px] font-bold">{r.title}</div>
              <div className="mt-1 text-[12.5px] text-ink/60">{r.desc}</div>
            </div>
            <Toggle
              id={r.id}
              ariaLabel={r.title}
              checked={state[r.id]}
              onChange={(v) => setState((s) => ({ ...s, [r.id]: v }))}
            />
          </div>
        ))}
      </motion.div>

      <p className="mt-6 font-mono text-[11px] tracking-[0.06em] text-ink/40">
        当前为本地预览；写入 aurora-core 配置将在后续接入。
      </p>
    </>
  );
}
