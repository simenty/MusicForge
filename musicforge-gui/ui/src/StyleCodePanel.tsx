// 风格码卡片（X15 / 蓝图能力 #11）：展示当前曲目文件名中的 `[Y23-S01-E01-C01-C02-V00]` 块。
//
// I18N-7：后端（`style_code` 命令）只回**结构化数据 / 原始码**，字段标签（年份 / Year、
// 风格 / Style…）由本组件按当前 UI 语言渲染——服务端不产出中文显示文案，英文界面不会
// 弹中文标签。
//
// 未配置 codebook 时码名查不到，按 core 既定策略**回退原始码**（绝不编造），
// 卡片底部给出说明，不假装知道码的含义。
import { useEffect, useState } from "react";
import { IS_DESKTOP, styleCode, trackGenre, type StyleCodeDto } from "./api";
import { useLang } from "./i18n";

export default function StyleCodePanel({
  path,
  title,
  codebookPath,
  onClose,
}: {
  /** 当前曲目路径（无 → 直接显示"无码"） */
  path: string | null;
  /** 标题栏展示的曲名 */
  title: string;
  /** codebook 路径（设置项；空 = 未配置 → 显示原始码） */
  codebookPath?: string;
  onClose: () => void;
}) {
  const { t } = useLang();
  /** undefined = 解析中；null = 无前导码块；对象 = 解析结果 */
  const [code, setCode] = useState<StyleCodeDto | null | undefined>(undefined);
  const [err, setErr] = useState<string | null>(null);
  /** 文件里**现有**的 genre（与解析结果对照；null = 无标签，undefined = 未取） */
  const [genre, setGenre] = useState<string | null | undefined>(undefined);

  // 现有 genre 与风格码解析**并行**发起——二者互不依赖，串行只会白等一个 RTT。
  useEffect(() => {
    if (!path || !IS_DESKTOP) {
      setGenre(undefined);
      return;
    }
    let alive = true;
    void trackGenre(path)
      .then((g) => {
        if (alive) setGenre(g);
      })
      .catch(() => {
        if (alive) setGenre(null); // 读不了按「无」展示，不阻断卡片
      });
    return () => {
      alive = false;
    };
  }, [path]);

  useEffect(() => {
    if (!path || !IS_DESKTOP) {
      setCode(null);
      return;
    }
    let alive = true;
    setCode(undefined);
    setErr(null);
    void styleCode(path, codebookPath)
      .then((r) => {
        if (alive) setCode(r);
      })
      .catch((e: unknown) => {
        if (alive) {
          setErr(String(e));
          setCode(null);
        }
      });
    return () => {
      alive = false;
    };
  }, [path, codebookPath]);

  // Esc 关闭（惯例同 LyricsPanel / ConfirmDialog）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 结构化数据 → 展示行（空值字段不出行：绝不为凑版面显示"—"）。
  // 码名：codebook 查到用码名，查不到**回退原始码**（绝不编造）。
  const name = (c: string) => code?.labels[c] ?? c;
  const rows: { label: string; value: string }[] = [];
  if (code) {
    if (code.year !== null) rows.push({ label: t.styleCode.year, value: String(code.year) });
    if (code.style) rows.push({ label: t.styleCode.style, value: name(code.style) });
    if (code.mood) rows.push({ label: t.styleCode.mood, value: name(code.mood) });
    if (code.scenes.length > 0) {
      rows.push({ label: t.styleCode.scene, value: code.scenes.map(name).join(" · ") });
    }
    if (code.version) rows.push({ label: t.styleCode.version, value: name(code.version) });
    for (const o of code.other) rows.push({ label: t.styleCode.other, value: o });
    // 文件**现有** genre：写回前先看现状，避免盲改。无标签显式说"（无）"，不留空白；
    // undefined = 还没取回来，显示占位而非假装"无"。
    rows.push({
      label: t.styleCode.currentGenre,
      value: genre === undefined ? "…" : (genre ?? t.styleCode.currentGenreNone),
    });
  }

  return (
    <div className="modal-mask" role="presentation" onClick={onClose}>
      <div
        className="modal sc-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="sc-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="sc-title">{t.styleCode.title(title)}</h3>
        </div>
        <div className="modal-body">
          {code === undefined ? (
            <p className="scan-note">{t.styleCode.loading}</p>
          ) : err ? (
            <p className="scan-error">{err}</p>
          ) : rows.length === 0 ? (
            <p className="scan-note">{t.styleCode.none}</p>
          ) : (
            <dl className="sc-card">
              {rows.map((r, i) => (
                <div className="sc-row" key={`${r.label}-${i}`}>
                  <dt>{r.label}</dt>
                  <dd>{r.value}</dd>
                </div>
              ))}
            </dl>
          )}
          {/* 仅在**未配置** codebook 时提示（配了还显示原始码说明该码未收录，非配置问题） */}
          {rows.length > 0 && !codebookPath && (
            <p className="scan-note sc-hint">{t.styleCode.rawHint}</p>
          )}
        </div>
        <div className="modal-foot">
          <button type="button" className="btn sm" onClick={onClose}>
            {t.player.close}
          </button>
        </div>
      </div>
    </div>
  );
}
