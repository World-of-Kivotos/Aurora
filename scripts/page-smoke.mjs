// Aurora 界面可见性冒烟（零依赖，需要先 `npm run dev` 起着 vite）。
//
//   node scripts/page-smoke.mjs
//
// 为什么要有这个脚本：五道关（tsc / vitest / vite build / clippy / cargo test）全绿的同时，
// 设置页在切过一次页签之后整片正文是不可见的——内容在 DOM 里、滚动条也在，就是 opacity 恒为 0。
// 类型对、测试过、构建成，因为出事的是「framer-motion 变体编排在运行时到底有没有到达子元素」，
// 这件事只有真跑一个 Chromium、真点一下页签、再去读计算样式才看得见。
//
// 于是这个脚本做的就是这件事：挂 CDP 驱动无头 Chromium，逐路由、逐页签点过去，等动效落定，
// 然后找出「占着版面却完全透明」的元素。判据取 offsetHeight > 0 && opacity === 0：
// 真心要藏的东西会用 display:none / hidden 属性 / 零高度，不会既占位又透明。
//
// 浏览器里没有 Tauri 注入的 __TAURI_INTERNALS__，前端会自动走 lib/ipc-mock 的假数据（见 tauri-bridge.ts），
// 所以每一页都有内容可渲染，不需要真后端。
//
// 退出码：发现问题为 1，干净为 0。

import { spawn } from "node:child_process";

const DEV_URL = process.env.AURORA_DEV_URL ?? "http://localhost:1420";
const PORT = Number(process.env.AURORA_CDP_PORT ?? 9333);
const CHROME =
  process.env.AURORA_CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";
const PROFILE = process.env.AURORA_CHROME_PROFILE ?? `${process.env.TEMP}/aurora-page-smoke`;

// 动效用的是 spring，没有固定时长；这里给的是「肉眼早已落定」的余量，不是精确等待。
const SETTLE_MS = 1200;

const ROUTES = [
  { hash: "#/", name: "启动屏" },
  { hash: "#/account", name: "账户" },
  { hash: "#/download", name: "下载" },
  { hash: "#/instance", name: "卷宗" },
  { hash: "#/settings", name: "设置" },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * 找出「整块死掉」的区域：一个容器的每一个占位子元素都完全透明。
 *
 * 判据刻意不是「存在占位却透明的元素」——那条太宽，界面里有一批元素本来就该透明：
 * 主操作键把 Start / Download / Retry 三个标签绝对定位叠在一起交叉淡入（键宽才不会跳），
 * 壁纸格子的删除按钮悬停才浮现。它们的共同点是身边总有可见的同辈。
 * 真出事的那种（切页签后整片正文停在 hidden）则相反：容器里一个可见的都不剩。
 * 按「全体透明」判，前者天然出局，后者一抓一个准。
 *
 * 命中即止不再下钻：一块死区的内部当然也是全透明的，逐层报只会把一条问题铺成几十行。
 */
const SCAN = `(() => {
  const root = document.querySelector('main[data-app-content]');
  if (!root) return { error: '没找到 main[data-app-content]' };
  const laidOut = (el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return null;
    if (el.offsetHeight <= 0 && el.offsetWidth <= 0) return null;
    return style;
  };
  const bad = [];
  const walk = (el) => {
    const kids = [...el.children].map((c) => [c, laidOut(c)]).filter(([, s]) => s !== null);
    if (kids.length > 0 && kids.every(([, s]) => Number(s.opacity) === 0)) {
      bad.push({
        tag: el.tagName.toLowerCase(),
        cls: el.className.toString().slice(0, 60),
        kids: kids.length,
        transform: kids[0][1].transform,
        text: (el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 48),
      });
      return;
    }
    for (const [child] of kids) walk(child);
  };
  walk(root);
  return { bad };
})()`;

/**
 * 列出当前页所有页签的文字。
 *
 * 不能只收 aria-current / aria-pressed 命中的那个——那是「当前选中」的标记，只有一个，
 * 拿它去点等于点了个寂寞，页签根本没切换过，正是这个脚本要抓的那个 bug 的触发条件没被满足。
 * 所以先由选中态定位到页签容器，再把容器里所有并列的按钮全取出来。
 */
const TAB_LABELS = `(() => {
  const root = document.querySelector('main[data-app-content]');
  if (!root) return [];
  const current = [...root.querySelectorAll('[aria-current], [aria-pressed]')]
    .map((el) => el.closest('button'))
    .filter(Boolean);
  const labels = [];
  for (const btn of current) {
    const group = btn.parentElement;
    if (!group) continue;
    for (const sibling of group.querySelectorAll(':scope > button')) {
      const text = sibling.textContent.trim();
      if (text.length > 0 && text.length < 12) labels.push(text);
    }
  }
  return [...new Set(labels)];
})()`;

const chrome = spawn(
  CHROME,
  [
    "--headless=new",
    `--remote-debugging-port=${PORT}`,
    `--user-data-dir=${PROFILE}`,
    "--no-first-run",
    "--window-size=1280,900",
    "about:blank",
  ],
  { stdio: "ignore" },
);

async function attach() {
  for (let i = 0; i < 40; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page;
    } catch {
      // 浏览器还没把调试端口打开，继续等
    }
    await sleep(250);
  }
  throw new Error(`CDP 未就绪（端口 ${PORT}）`);
}

const page = await attach();
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => {
  ws.onopen = res;
  ws.onerror = rej;
});

let seq = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  const slot = pending.get(msg.id);
  if (!slot) return;
  pending.delete(msg.id);
  msg.error ? slot.rej(new Error(JSON.stringify(msg.error))) : slot.res(msg.result);
};
const send = (method, params = {}) =>
  new Promise((res, rej) => {
    const id = ++seq;
    pending.set(id, { res, rej });
    ws.send(JSON.stringify({ id, method, params }));
  });

async function evaluate(expression) {
  const result = await send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}

await send("Page.enable");
await send("Runtime.enable");

let failures = 0;

/** 报一处，并计入失败。 */
function report(where, bad) {
  failures += bad.length;
  console.log(`  FAIL ${where}：${bad.length} 块区域整体不可见`);
  for (const item of bad) {
    console.log(`       <${item.tag} class="${item.cls}"> ${item.kids} 个子元素全透明 ${item.transform}`);
    console.log(`       文本：${item.text}`);
  }
}

for (const route of ROUTES) {
  // 每条路由都整页重载：只改 hash 不会重挂载外壳，路由间的残留状态会互相污染判据。
  await send("Page.navigate", { url: `${DEV_URL}/${route.hash}` });
  await sleep(SETTLE_MS * 2);

  const first = await evaluate(SCAN);
  if (first.error) {
    console.log(`${route.name}：${first.error}`);
    failures += 1;
    continue;
  }
  console.log(`${route.name} ${route.hash}`);
  if (first.bad.length > 0) report("首屏", first.bad);
  else console.log("  ok 首屏");

  const tabs = await evaluate(TAB_LABELS);
  for (const label of tabs) {
    const clicked = await evaluate(`(() => {
      const btn = [...document.querySelectorAll('main[data-app-content] button')]
        .find((b) => b.textContent.trim() === ${JSON.stringify(label)});
      if (!btn) return false;
      btn.click();
      return true;
    })()`);
    if (!clicked) continue;
    await sleep(SETTLE_MS);
    const after = await evaluate(SCAN);
    if (after.bad?.length > 0) report(`页签「${label}」`, after.bad);
    else console.log(`  ok 页签「${label}」`);
  }
}

ws.close();
chrome.kill();

console.log(failures === 0 ? "\n全部页面可见性正常" : `\n共 ${failures} 处异常`);
process.exit(failures === 0 ? 0 : 1);
