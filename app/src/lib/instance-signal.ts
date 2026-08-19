// 「实例集合可能变了」的进程内广播。
//
// 起因是侧栏的「管理」入口：它要知道那个唯一实例是否已就位，但侧栏是窗口外壳的常驻件，
// 不随页面重挂。原先跟着 pathname 重探一次，前提是「装完游戏必然换页」——这个前提是错的：
// 启动屏的 installGame 装完只 load() 刷新自身，从不 navigate，玩家原地就能点 Start 开始玩，
// pathname 一直停在 "/"，入口于是永远长不出来。所以改由写入方在改动选中实例之后显式广播。
//
// 不引状态库、不用自定义 DOM 事件：要传的只是「第几次变更」这一个数，
// React 自带的 useSyncExternalStore 就是为这种外部可变源准备的。

let revision = 0;
const listeners = new Set<() => void>();

/** 选中实例或已安装集合发生变化后调用。写入方负责在后端已落盘之后再广播。 */
export function notifyInstanceChanged(): void {
  revision += 1;
  listeners.forEach((listener) => listener());
}

export function subscribeInstanceChanged(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * 快照即变更计数。服务端渲染同样返回它：静态渲染不跑副作用，读到的必是初值，
 * 与客户端首帧一致，不会引起水合不匹配。
 */
export function instanceChangeRevision(): number {
  return revision;
}
