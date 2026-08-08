// node --test scripts/
//
// 覆盖三件容易在发版当天出事的判断：分支到频道的映射（发错频道 = 稳定版用户看到开发版文案）、
// 缺文案时必须报错而不是静默放行、以及只有空白的文案要按缺失处理。

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { channelOf, locate, requireNotes, RELEASES_DIR } from "./release-notes.mjs";
import { tauriVersion } from "./shared.mjs";

test("main 与热修分支归稳定频道，其余归开发频道", () => {
  assert.equal(channelOf("main"), "main");
  assert.equal(channelOf("hotfix/1.2.4-闪退"), "main");
  assert.equal(channelOf("dev"), "dev");
  assert.equal(channelOf("feat/mod-依赖预览"), "dev");
  // 「mainline」不是 main：拿前缀匹配写会把它误判成稳定频道。
  assert.equal(channelOf("mainline"), "dev");
  // 「hotfix」不带斜杠也不算：热修必须是 hotfix/<描述> 的形式。
  assert.equal(channelOf("hotfix"), "dev");
});

test("路径按频道与版本号拼，版本号默认取 tauri.conf.json", () => {
  const found = locate("dev", "1.2.3");
  assert.equal(found.channel, "dev");
  assert.equal(found.version, "1.2.3");
  assert.equal(found.file, join(RELEASES_DIR, "dev", "1.2.3.md"));

  assert.equal(locate("main").version, tauriVersion());
  assert.equal(locate("main", "1.2.3").file, join(RELEASES_DIR, "main", "1.2.3.md"));
});

test("缺文案时报错，且错误里点名缺的是哪个文件", () => {
  assert.throws(
    () => requireNotes("dev", "99.99.99"),
    (e) => e.message.includes("docs/releases/dev/99.99.99.md"),
  );
});

test("只有空白的文案按缺失处理，不放行", () => {
  const base = mkdtempSync(join(tmpdir(), "aurora-notes-"));
  try {
    mkdirSync(join(base, "dev"), { recursive: true });
    writeFileSync(join(base, "dev", "1.2.3.md"), "\n   \n\t\n", "utf8");
    assert.throws(
      () => requireNotes("dev", "1.2.3", base),
      (e) => e.message.includes("是空的"),
    );

    // 有内容就该读出来，并且首尾空白被剥掉——否则 latest.json 的 notes 会带尾随空行。
    writeFileSync(join(base, "dev", "1.2.3.md"), "\n- 修好了闪退\n\n", "utf8");
    assert.equal(requireNotes("dev", "1.2.3", base).notes, "- 修好了闪退");
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("仓库里当前版本的两个频道文案都在且非空", () => {
  const version = tauriVersion();
  for (const ref of ["main", "dev"]) {
    const found = requireNotes(ref);
    assert.equal(found.version, version);
    assert.ok(found.notes.length > 0, `${ref} 频道文案为空`);
    assert.equal(found.notes, found.notes.trim());
  }
});
