// 设置持久化：localStorage（本地存储，零网络——不向任何外部写入）
// WebView2 在极少数配置下可能禁用 localStorage，所有读写都吞掉异常并回退到默认值，
// 绝不让持久化失败影响主流程。

export type SaveTo = "source" | "custom";

export interface Settings {
  /** 输出位置模式：源目录 or 自定义目录 */
  saveTo: SaveTo;
  /** 自定义输出目录（saveTo === "custom" 时有效） */
  outDir: string;
  /** 命名模板 */
  template: string;
  /** 跳过已存在且通过完整性校验的输出 */
  skipExisting: boolean;
  /** 目录输入是否递归 */
  recursive: boolean;
  /** 并发数 */
  jobs: number;
  /** 仅规划（dry-run）：产出 manifest 计划，不写音频/侧车 */
  dryRun: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  saveTo: "source",
  outDir: "",
  template: "{artist}/{album}/{track:02d} {title}",
  skipExisting: true,
  recursive: true,
  jobs: 4,
  dryRun: false,
};

const KEY = "musicforge.settings.v1";

export function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const parsed = JSON.parse(raw) as Partial<Settings>;
    // 逐字段校验：任何非法值都回退到默认，避免旧版本/损坏数据把 UI 带进坏状态
    const jobs =
      typeof parsed.jobs === "number" && Number.isFinite(parsed.jobs)
        ? Math.min(16, Math.max(1, Math.round(parsed.jobs)))
        : DEFAULT_SETTINGS.jobs;
    return {
      saveTo: parsed.saveTo === "custom" ? "custom" : "source",
      outDir: typeof parsed.outDir === "string" ? parsed.outDir : "",
      template:
        typeof parsed.template === "string" && parsed.template.trim()
          ? parsed.template
          : DEFAULT_SETTINGS.template,
      skipExisting:
        typeof parsed.skipExisting === "boolean"
          ? parsed.skipExisting
          : DEFAULT_SETTINGS.skipExisting,
      recursive:
        typeof parsed.recursive === "boolean"
          ? parsed.recursive
          : DEFAULT_SETTINGS.recursive,
      dryRun:
        typeof parsed.dryRun === "boolean" ? parsed.dryRun : DEFAULT_SETTINGS.dryRun,
      jobs,
    };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(s: Settings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // 持久化失败不应影响使用
  }
}
