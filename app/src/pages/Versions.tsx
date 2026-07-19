// 版本管理：已安装实例列表 + 设为当前启动版本。单独成页，后续在此扩展实例详情（Mod / 存档 / 独立设置 / 诊断）。
// 安装新版本在「下载」页；此页只管「管理已有的」。

import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { EmptyState } from "../components/EmptyState";
import { Button } from "../components/Button";
import { useToast } from "../components/Toast";
import { AlertIcon, LayersIcon, RefreshIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  getConfig,
  listInstalled,
  updateConfig,
  type InstalledVersionDto,
  type VersionScanDto,
} from "../lib/ipc";

function splitId(id: string) {
  const i = id.indexOf("-");
  return i < 0 ? { base: id, sfx: "" } : { base: id.slice(0, i), sfx: id.slice(i) };
}
function loaderText(v: InstalledVersionDto) {
  const l = v.loaders[0];
  return !l ? "原版" : l.version ? `${l.kind} ${l.version}` : l.kind;
}

export function Versions() {
  const { toast } = useToast();
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [sc, cfg] = await Promise.all([listInstalled(), getConfig()]);
      setScan(sc);
      setSelected(cfg.selected_version);
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const versions = scan?.versions ?? [];
  const broken = scan?.broken ?? [];
  const current = selected && versions.some((v) => v.id === selected) ? selected : (versions[0]?.id ?? null);

  const setAsCurrent = async (id: string) => {
    const prev = selected;
    setSelected(id);
    try {
      await updateConfig({ selectedVersion: id });
      toast("已设为当前启动版本", "success");
    } catch (e) {
      setSelected(prev);
      toast(String(e), "error");
    }
  };

  return (
    <>
      <motion.div variants={pageItem} className="mb-6 flex items-baseline justify-between">
        <div className="flex items-baseline gap-4">
          <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">版本</h1>
          <span className="text-[12px] text-ink/35">管理已安装的实例，点选即设为当前启动版本</span>
        </div>
        <button
          type="button"
          onClick={() => void load()}
          className="inline-flex items-center gap-1.5 text-[12px] font-semibold text-ink/45 transition-colors hover:text-ink [&_svg]:h-3.5 [&_svg]:w-3.5"
        >
          <RefreshIcon />
          刷新
        </button>
      </motion.div>

      {error && (
        <motion.div
          variants={pageItem}
          className="mb-5 flex items-center gap-3 rounded-[3px] border border-danger/40 px-4 py-3 text-[13px] text-danger"
        >
          <AlertIcon size={18} />
          <span className="flex-1">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void load()}>
            重试
          </Button>
        </motion.div>
      )}

      {versions.length + broken.length === 0 ? (
        <motion.div variants={pageItem}>
          <EmptyState icon={<LayersIcon />} title="还没有安装任何版本，去「下载」装一个" />
        </motion.div>
      ) : (
        <motion.ul variants={pageItem} className="m-0 list-none p-0">
          {versions.map((v) => {
            const s = splitId(v.id);
            const isCur = v.id === current;
            return (
              <li key={v.id}>
                <button
                  type="button"
                  onClick={() => void setAsCurrent(v.id)}
                  aria-pressed={isCur}
                  title="设为当前启动版本"
                  className="flex w-full items-center justify-between gap-6 border-b border-ink/8 py-[15px] text-left transition-colors last:border-b-0 hover:bg-ink/[0.03]"
                >
                  <span className="flex items-center gap-3">
                    <span className="text-[21px] font-bold tracking-[-0.01em] tabular-nums">{s.base}</span>
                    {s.sfx && <span className="text-[14px] font-semibold text-ink/35">{s.sfx}</span>}
                    {isCur && (
                      <span className="rounded-[2px] bg-accent/12 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em] text-accent">
                        当前
                      </span>
                    )}
                  </span>
                  <span className="flex shrink-0 items-center gap-4 text-[12px] text-ink/40">
                    <span>{v.mc_version !== v.id ? v.mc_version : ""}</span>
                    <span className="font-mono">{loaderText(v)}</span>
                  </span>
                </button>
              </li>
            );
          })}
          {broken.map((b) => (
            <li key={b.id}>
              <div className="flex items-center justify-between gap-6 border-b border-ink/8 py-[15px] last:border-b-0">
                <span className="text-[21px] font-bold text-danger tabular-nums">{b.id}</span>
                <span className="flex items-center gap-2 text-[12px] text-danger/80">
                  <span className="rounded-[2px] border border-danger/50 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em]">
                    损坏
                  </span>
                  <span className="font-mono">{b.reason}</span>
                </span>
              </div>
            </li>
          ))}
        </motion.ul>
      )}
    </>
  );
}
