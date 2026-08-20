// framer-motion 弹簧预设与页面入场变体——集中一处复用，页面/组件不得各自另写弹簧参数。
// 两参弹簧直接映射 framer-motion 的 { type:"spring", duration, bounce }（对应 spec 第八节动效）。

import type { Transition, Variants } from "framer-motion";

export type SpringPreset = "tap" | "settle" | "pop" | "soft" | "morph" | "aurora";

export const springs: Record<SpringPreset, Transition> = {
  tap: { type: "spring", duration: 0.22, bounce: 0 },
  settle: { type: "spring", duration: 0.26, bounce: 0.1 },
  pop: { type: "spring", duration: 0.32, bounce: 0.2 },
  soft: { type: "spring", duration: 0.34, bounce: 0.12 },
  morph: { type: "spring", duration: 0.35, bounce: 0.14 },
  // 默认 aurora 弹簧：过冲最明显，用于强调性入场/切换。
  aurora: { type: "spring", duration: 0.3, bounce: 0.35 },
};

// 页面容器：对直接子级 motion 元素做轻微 stagger（子级用 pageItem 变体即自动排队上滑）。
export const pageContainer: Variants = {
  hidden: {},
  show: {
    transition: { staggerChildren: 0.05, delayChildren: 0.02 },
  },
};

// 页面分区/卡片入场：透明 + 8px 上滑，用 soft 弹簧落位。
export const pageItem: Variants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: springs.soft },
};

/**
 * 页签正文的切换过渡：设置 / 下载 / 卷宗三页共用一份。
 *
 * 必须写成变体标签，不能写成 initial={{...}} animate={{...}} 的对象字面量——这不是风格问题，
 * 写成对象会让内部所有带 variants 的子元素永久停在 hidden（表现为切一次页签整片正文变透明，
 * 内容还在 DOM 里、滚动条也还在，就是看不见）。成因：
 *
 *   1. 带 variants 的子元素属于「等父级变体节点来编排」的继承子级，自己不主动播放；
 *      唯一的例外是 framer-motion 的 manuallyAnimateOnMount，它取 Boolean(parent && parent.current)。
 *   2. AnimatePresence 换 key 时，这块面板与它内部的子元素在同一次提交里挂载。子元素的
 *      VisualElement 在 render 阶段就构造，那时面板的 DOM 还没落地，parent.current 是 null，
 *      于是 manuallyAnimateOnMount 为 false，这条自救通道关闭。
 *   3. 面板若不是变体节点，getClosestVariantNode 会越过它、把子元素挂到更上层那个变体节点上；
 *      而上层只在页面挂载时编排过一次，此后不会因为换页签再编排一轮。编排永远不会到来。
 *
 * 首屏之所以正常，是 AnimatePresence 的 initial={false} 让子元素直接以 animate 态渲染，
 * 绕过了整个编排链路——所以这个坑只在「切过一次页签」之后才现形，五道关一道也拦不住。
 *
 * 标签名必须与 pageItem 同名（hidden / show），子元素才接得住这次编排。
 */
export const tabPanel: Variants = {
  hidden: { opacity: 0, y: 6 },
  show: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -4 },
};
