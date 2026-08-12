// 主页的整版背景。
//
// 铺满整个窗口，外壳（标题栏、侧栏）浮在图上并改用磨砂纸（见 app.css 的 paper-frost）。
// 早先只铺内容区、外壳保持不透明纸色，理由是「纸色不透明才让导航永远可读」——
// 可读性这条没错，但代价是左边一堵纸墙、顶上一条纸带与图硬碰硬，
// 图看起来像贴进窗口里的一张画，而不是这个应用的底。磨砂同时满足两者：
// 图透上来所以没有缝，纸色占七成八所以字照样清楚。
// 其余页面（下载/版本/设置）信息密度高，图只会干扰，仍然不铺——封面用整版图，内页回到纸面。
//
// 三层自下而上：兜底纯色 → 图 → 纸色遮罩。
//
// 定位是相对于外壳根节点的 absolute，覆盖整个窗口矩形。它不在滚动容器里，因此不随内容卷走。

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
  /** 是否铺压暗层。仅当右下角改用纸色裸字时为真，由外壳按取样结果判定。 */
  scrim: boolean;
}

export function PageBackground({ file, tint, veil, scrim }: Props) {
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

          {/* 压暗层压在柔化之上：柔化会把底色提亮，字色判定按「图 -> 柔化 -> 压暗」这个顺序算，
              图层顺序必须跟着一致，否则算出来的对比度对不上实际看到的。 */}
          {scrim && <div className="plate-scrim absolute inset-0" />}

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
