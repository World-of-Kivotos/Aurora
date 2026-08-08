// 发行说明的定位与读取。
//
// 文案按「频道 + 版本号」存在 docs/releases/<channel>/<version>.md，CI 据当前分支与
// tauri.conf.json 的版本号取对应那一份，同时喂给 GitHub Release 的正文和 latest.json
// 的 notes 字段——后者会直接显示在用户的更新弹窗里。
//
// 缺文件一律非零退出，不给占位文案兜底：装出一个说明写着「Aurora 0.1.0」的包，
// 等于把「忘了写更新说明」这件事留到用户那边才暴露。
//
//   node scripts/release-notes.mjs resolve <分支名>        输出 JSON：channel/version/file
//   node scripts/release-notes.mjs print <分支名>          把文案打到 stdout
//   node scripts/release-notes.mjs emit <分支名> <输出路径>  把文案写到文件（CI 用，免去多行转义）

import { existsSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { pathToFileURL } from "node:url";
import { repoRoot, readText, tauriVersion } from "./shared.mjs";

export const RELEASES_DIR = join(repoRoot, "docs", "releases");

/**
 * 分支名到发布频道。
 *
 * 热修走单开分支合 main 的路子，所以它面向的是稳定版用户，文案该取 main 那一份；
 * 其余分支（含 dev 本身与各种特性分支）统一按开发版处理。
 */
export function channelOf(refName) {
  if (refName === "main" || refName.startsWith("hotfix/")) return "main";
  return "dev";
}

/** 定位文案文件。不做存在性检查，校验交给 requireNotes。baseDir 只为测试留口。 */
export function locate(refName, version = tauriVersion(), baseDir = RELEASES_DIR) {
  const channel = channelOf(refName);
  return { channel, version, file: join(baseDir, channel, `${version}.md`) };
}

/** 取文案内容；文件不存在或只有空白则抛错。 */
export function requireNotes(refName, version = tauriVersion(), baseDir = RELEASES_DIR) {
  const found = locate(refName, version, baseDir);
  const shown = relative(repoRoot, found.file).replaceAll("\\", "/");
  if (!existsSync(found.file)) {
    throw new Error(
      `缺少发行说明 ${shown}。发 ${found.version} 之前先补上这份文案，` +
        `参照 docs/releases/README.md 的写法。`,
    );
  }
  const notes = readText(found.file).trim();
  if (notes === "") throw new Error(`发行说明 ${shown} 是空的，没有内容可以发给用户。`);
  return { ...found, shown, notes };
}

function main() {
  const [cmd, refName, outPath] = process.argv.slice(2);
  // CI 上不传分支名时退回 Actions 注入的环境变量，本地则必须显式给，免得默认成 dev 发错频道。
  const ref = refName ?? process.env.GITHUB_REF_NAME;
  if (!cmd || !ref) {
    console.error("用法：node scripts/release-notes.mjs <resolve|print|emit> <分支名> [输出路径]");
    process.exit(1);
  }

  const found = requireNotes(ref);
  switch (cmd) {
    case "resolve":
      console.log(JSON.stringify({ channel: found.channel, version: found.version, file: found.shown }));
      break;
    case "print":
      console.log(found.notes);
      break;
    case "emit":
      if (!outPath) {
        console.error("emit 需要输出路径：node scripts/release-notes.mjs emit <分支名> <输出路径>");
        process.exit(1);
      }
      writeFileSync(outPath, found.notes + "\n", "utf8");
      console.log(`${found.shown} -> ${outPath}（${found.channel} 频道，${found.version}）`);
      break;
    default:
      console.error(`未知命令 ${cmd}`);
      process.exit(1);
  }
}

// 被测试 import 时不执行 main。
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  main();
}
