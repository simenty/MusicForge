// 体积护栏（P6.13）：区分「首屏静态链」与「懒加载 chunk」。
//
// 为什么两个口径：
// - 只看总体积会掩盖首屏变胖——页面代码与外壳代码被同等计价，而用户只在启动
//   时为「首屏」付费（懒加载块是进入页面时才取）；
// - 只看入口又会漏掉懒加载块的膨胀——页面越堆越大却无人报警。
// manifest（vite build 产出）给出入口的静态导入链，据此算首屏，其余归按需加载。
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const DIST = "dist";
const ASSETS = join(DIST, "assets");
const KB = 1024;
/** 首屏预算：启动时必须下载并解析的部分 */
const INITIAL_BUDGET = 300 * KB;
/** 总体积预算：安装包携带的全部 JS（含按需加载块） */
const TOTAL_BUDGET = 420 * KB;

const size = (p) => statSync(p).size;
const fmt = (n) => `${(n / KB).toFixed(0)}KB`;

// 历史 bug 回归：styles.css 未被 import → 产物里一个 CSS 都没有（界面裸奔）
const css = readdirSync(ASSETS).filter((f) => f.endsWith(".css"));
if (css.length === 0) {
  console.error("X CSS 产物缺失: styles.css 可能未被 import（历史 bug，勿回归）");
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(join(DIST, ".vite", "manifest.json"), "utf8"));
const entry = manifest["index.html"];
if (!entry) {
  console.error("X manifest 缺 index.html 入口——检查 vite.config.ts（build.manifest 须为 true）");
  process.exit(1);
}

// 首屏 = 入口 + 其静态导入链（递归，去重）
const byFile = new Map(Object.values(manifest).map((m) => [m.file, m]));
// manifest 的 `imports` 引用的是**键**：非入口/页面块键 == 其 `file`，但 manualChunks
// 等共享块键带前导 `_` 且不含 `assets/`（`_react-vendor-xxx.js` vs `assets/react-vendor-xxx.js`）。
// 直接用键做 `size(join(DIST, key))` 会因路径错位 ENOENT——P2-22 引入 manualChunks 触发。
// 建「键→file」映射，walk 时把键解析成真实产出路径。
const keyToFile = new Map(Object.entries(manifest).map(([k, m]) => [k, m.file]));
const seen = new Set();
const initialFiles = [];
const walk = (file) => {
  if (seen.has(file)) return;
  seen.add(file);
  initialFiles.push(file);
  for (const imp of byFile.get(file)?.imports ?? []) walk(keyToFile.get(imp) ?? imp);
};
walk(entry.file);
const initial = initialFiles.reduce((n, f) => n + size(join(DIST, f)), 0);

const allJs = readdirSync(ASSETS).filter((f) => f.endsWith(".js"));
const total = allJs.reduce((n, f) => n + size(join(ASSETS, f)), 0);
const lazyCount = allJs.length - initialFiles.length;

console.log(
  `产物 OK: CSS ${css.join(", ")} / JS ${allJs.length} 块（首屏 ${initialFiles.length} + 懒加载 ${lazyCount}）`
);
console.log(
  `  首屏 ${fmt(initial)} / 预算 ${fmt(INITIAL_BUDGET)}    总体积 ${fmt(total)} / 预算 ${fmt(TOTAL_BUDGET)}`
);

let bad = false;
if (initial > INITIAL_BUDGET) {
  console.error(
    `X 首屏超预算: ${fmt(initial)} > ${fmt(INITIAL_BUDGET)}——新代码进了主包：页面应走 lazy()` +
      `，共享件（i18n / api / 图标）入主包前先评估体积`
  );
  bad = true;
}
if (total > TOTAL_BUDGET) {
  console.error(`X 总体积超预算: ${fmt(total)} > ${fmt(TOTAL_BUDGET)}——检查依赖或按页面继续切分`);
  bad = true;
}
process.exit(bad ? 1 : 0);
