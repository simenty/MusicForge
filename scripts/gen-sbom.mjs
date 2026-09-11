#!/usr/bin/env node
// SBOM 生成（P4-4）：从 `cargo metadata` 提取全部依赖（workspace 成员 + 传递依赖），
// 输出 CycloneDX 1.5 JSON——**零第三方工具**（不引入 cargo-cyclonedx/syft）。
//
// 目的（v3 审计 §5.5）：发布产物附 SBOM，让用户/审计方可核对二进制内究竟含什么。
// Cargo.lock 已锁定精确版本，`--locked` 保证 SBOM 与实际构建一致（可复现）。
//
// 用法: node scripts/gen-sbom.mjs [version] > SBOM.cdx.json
import { execFileSync } from "node:child_process";

const version = process.argv[2] || "0.0.0";

let meta;
try {
  meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
      maxBuffer: 128 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    }).toString()
  );
} catch (e) {
  console.error("X cargo metadata 失败：", e.message);
  process.exit(1);
}

const seen = new Set();
const components = [];
for (const p of meta.packages) {
  const key = `${p.name}@${p.version}`;
  if (seen.has(key)) continue;
  seen.add(key);
  const c = { type: "library", name: p.name, version: p.version };
  // license 可能是 SPDX 表达式（如 "MIT OR Apache-2.0"）→ 用 expression 形式
  if (p.license) c.licenses = [{ expression: p.license }];
  if (p.repository) c.externalReferences = [{ type: "vcs", url: p.repository }];
  components.push(c);
}
components.sort((a, b) =>
  `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`)
);

const bom = {
  bomFormat: "CycloneDX",
  specVersion: "1.5",
  version: 1,
  metadata: {
    timestamp: new Date().toISOString(),
    component: { type: "application", name: "MusicForge", version },
    tools: [{ name: "scripts/gen-sbom.mjs", vendor: "MusicForge" }],
  },
  components,
};
process.stdout.write(JSON.stringify(bom, null, 2) + "\n");
