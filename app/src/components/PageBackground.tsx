// 主页的整版背景。
//
// 只铺内容区，不铺侧栏与标题栏：那两处是应用外壳，纸色不透明才让导航永远可读；
// 内容区是版面，图进版面。其余页面（下载/版本/设置）信息密度高，图只会干扰，故不铺。
//
// 三层自下而上：兜底纯色 → 图 → 纸色遮罩。
//
// 定位是相对于 main 这个滚动容器的 absolute，所以图铺的是整个可滚动区域而非可视区。
// 主页内容不足一屏、不产生滚动，两者等价；哪天主页要能滚了，这里得改成 sticky 才不会跟着卷走。

import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useState } from "react";
import { currentBackgroundUrl } from "../lib/appearance";
import { useMotionPref } from "../lib/motion-pref";

interface Props {
  /** 当前背景文件名；null 表示纯纸面，本组件整体不渲染。 */
  file: string | null;
  /** 背景平均色，图解码完成前先铺它。 */
  tint: string | null;
  /** 纸色遮罩强度（百分比）。 */
  veil: number;
}

export function PageBackground({ file, tint, veil }: Props) {
  // MotionConfig 已全局接管 framer-motion 的动效降级，但下面那张 img 的淡入是 CSS transition，
  // 不在它管辖内，所以这里要自己读一次偏好。
  const { reduceMotion } = useMotionPref();
  // 图解码要时间，先让兜底色占位，onLoad 再把图淡进来——直接渲染 img 会先闪一下纸底。
  const [loaded, setLoaded] = useState(false);

  // 换图后重新走一遍淡入；不重置的话新图会硬切。
  useEffect(() => setLoaded(false), [file]);

  return (
    <AnimatePresence>
      {file && (
        <motion.div
          key={file}
          aria-hidden
          className="pointer-events-none absolute inset-0 overflow-hidden"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduceMotion ? 0 : 0.35 }}
        >
          {/* 兜底色：图没解码完时就是它，避免开机闪白。 */}
          <div className="absolute inset-0" style={{ background: tint ?? "var(--color-paper)" }} />

          <img
            src={currentBackgroundUrl(stampOf(file))}
            alt=""
            onLoad={() => setLoaded(true)}
            className="absolute inset-0 h-full w-full object-cover transition-opacity"
            style={{
              opacity: loaded ? 1 : 0,
              transitionDuration: reduceMotion ? "0ms" : "500ms",
            }}
          />

          {/* 纸色遮罩：文字都在纸片上，可读性不靠它；这是给花图留的退路。 */}
          {veil > 0 && (
            <div
              className="absolute inset-0"
              style={{ background: "var(--color-paper)", opacity: veil / 100 }}
            />
          )}

          {/* 顶部不做渐隐。试过一道 64px 的纸色渐变去衔接标题栏，实拍下来是一条发灰的脏带子——
              杂志的跨页图本来就是硬边切到页边，图与标题栏之间那条干净的分界才是版面语言。 */}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/**
 * 由文件名派生一个稳定的缓存戳。
 *
 * 协议给图片打了 immutable 缓存头，换图必须换 URL 才能让 WebView 放弃旧的那份。
 * 用文件名的哈希而不是 Date.now()：后者每次渲染都变，等于每次进主页都重读一遍磁盘。
 */
function stampOf(file: string): number {
  let hash = 0;
  for (let i = 0; i < file.length; i++) {
    hash = (hash * 31 + file.charCodeAt(i)) | 0;
  }
  return hash >>> 0;
}
