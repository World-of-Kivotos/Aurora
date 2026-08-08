// 发版脚本的公共读取工具。
//
// 抽出来不是为了美观：BOM 这个坑已经踩过一次——用 PowerShell 的 Set-Content 改过
// package.json 后 JSON.parse 当场炸在第一个字符上。每个脚本各写一遍读文件逻辑，
// 漏掉剥 BOM 的那份就会复现同一个 bug。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

export const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

export const TAURI_CONF = join(repoRoot, "app", "src-tauri", "tauri.conf.json");

/** 读文本并剥掉 BOM。 */
export function readText(path) {
  return readFileSync(path, "utf8").replace(/^﻿/, "");
}

/** 读版本号的唯一事实来源：tauri.conf.json 的 version。 */
export function tauriVersion() {
  const conf = JSON.parse(readText(TAURI_CONF));
  if (typeof conf.version !== "string") {
    throw new Error("tauri.conf.json 缺少 version 字段");
  }
  return conf.version;
}
