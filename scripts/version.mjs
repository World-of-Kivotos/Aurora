// 版本号一致性工具。
//
// 唯一事实来源是 app/src-tauri/tauri.conf.json 的 version：它决定安装包版本号、
// 自更新比对的版本、以及产物文件名。另外两处（前端 package.json、Rust crate 的 Cargo.toml）
// 只是跟随者——发版时手改三处必然漏掉一个，所以这里提供机械校验与同步。
//
//   node scripts/version.mjs check   校验三处一致，不一致以非零码退出（CI 用）
//   node scripts/version.mjs sync    以 tauri.conf.json 为准改写另外两处
//   node scripts/version.mjs set 1.2.3   先改 tauri.conf.json 再同步

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_CONF = join(repoRoot, "app", "src-tauri", "tauri.conf.json");
const PACKAGE_JSON = join(repoRoot, "app", "package.json");
const CARGO_TOML = join(repoRoot, "app", "src-tauri", "Cargo.toml");

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

// 剥掉 BOM 再交给 JSON.parse：Windows 上用 PowerShell 的 Set-Content 或某些编辑器
// 改这些文件会留下 BOM，JSON.parse 会当场炸在第一个字符上。
const read = (p) => readFileSync(p, "utf8").replace(/^﻿/, "");

/** 读唯一事实来源。 */
function sourceVersion() {
  const conf = JSON.parse(read(TAURI_CONF));
  if (typeof conf.version !== "string") {
    throw new Error("tauri.conf.json 缺少 version 字段");
  }
  return conf.version;
}

/** Cargo.toml 的包版本：只认 [package] 段里第一个顶格 version，不碰依赖的版本号。 */
function cargoVersion(text) {
  const m = text.match(/^\s*\[package\][\s\S]*?^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error("Cargo.toml 里找不到 [package] 的 version");
  return m[1];
}

function writeCargoVersion(text, next) {
  // 只替换 [package] 段内的那一处：全局替换会把依赖项的版本号一起改掉。
  const pkgStart = text.search(/^\s*\[package\]/m);
  if (pkgStart < 0) throw new Error("Cargo.toml 里找不到 [package] 段");
  const head = text.slice(0, pkgStart);
  const rest = text.slice(pkgStart);
  const replaced = rest.replace(/^(\s*version\s*=\s*)"[^"]+"/m, `$1"${next}"`);
  if (replaced === rest) throw new Error("Cargo.toml 的 version 未能改写");
  return head + replaced;
}

function currentVersions() {
  return {
    tauri: sourceVersion(),
    pkg: JSON.parse(read(PACKAGE_JSON)).version,
    cargo: cargoVersion(read(CARGO_TOML)),
  };
}

function check() {
  const v = currentVersions();
  const bad = [];
  if (v.pkg !== v.tauri) bad.push(`app/package.json 是 ${v.pkg}`);
  if (v.cargo !== v.tauri) bad.push(`app/src-tauri/Cargo.toml 是 ${v.cargo}`);
  if (bad.length > 0) {
    console.error(`版本号不一致。以 tauri.conf.json 的 ${v.tauri} 为准，但 ${bad.join("，")}。`);
    console.error("跑 node scripts/version.mjs sync 同步。");
    process.exit(1);
  }
  console.log(`版本号一致：${v.tauri}`);
}

function sync() {
  const target = sourceVersion();

  const pkg = JSON.parse(read(PACKAGE_JSON));
  if (pkg.version !== target) {
    pkg.version = target;
    // package.json 用两空格缩进并保留末尾换行，避免每次同步都产生无谓的格式 diff。
    writeFileSync(PACKAGE_JSON, JSON.stringify(pkg, null, 2) + "\n", "utf8");
    console.log(`app/package.json -> ${target}`);
  }

  const cargo = read(CARGO_TOML);
  if (cargoVersion(cargo) !== target) {
    writeFileSync(CARGO_TOML, writeCargoVersion(cargo, target), "utf8");
    console.log(`app/src-tauri/Cargo.toml -> ${target}`);
  }

  console.log(`已同步到 ${target}`);
}

function set(next) {
  if (!SEMVER.test(next)) {
    console.error(`版本号格式不合法：${next}（要形如 1.2.3 或 1.2.3-beta.1）`);
    process.exit(1);
  }
  const conf = JSON.parse(read(TAURI_CONF));
  conf.version = next;
  writeFileSync(TAURI_CONF, JSON.stringify(conf, null, 2) + "\n", "utf8");
  console.log(`tauri.conf.json -> ${next}`);
  sync();
}

const [cmd, arg] = process.argv.slice(2);
switch (cmd) {
  case "check":
    check();
    break;
  case "sync":
    sync();
    break;
  case "set":
    if (!arg) {
      console.error("用法：node scripts/version.mjs set <版本号>");
      process.exit(1);
    }
    set(arg);
    break;
  default:
    // 无参数时打印当前状态，方便随手查。
    console.log(JSON.stringify(currentVersions(), null, 2));
}
