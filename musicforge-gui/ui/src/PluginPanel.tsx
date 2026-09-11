// P6a（X37）：「AI 与插件」功能态。
//
// v0.6.0 占位态 → v0.7.0 解锁：本面板展示
// 1. 插件运行时可用性（当前构建是否含 plugin-host）；
// 2. 白名单目录内已安装插件清单（PLUGIN_POLICY.md 的白名单，配置外路径不可见）；
// 3. 每个插件的启用开关（写 config.json `plugins.enabled`，X36）。
//
// 安全面（不变）：页面本身**零请求**——只经 IPC 调本地命令；URL 保持纯文本
// 展示（不可点击），不加载任何远程资源。降级提示常驻：本地五域功能无需插件。

import { useCallback, useEffect, useState } from "react";
import {
  IS_DESKTOP,
  formatMigrate,
  pluginsAcknowledge,
  pluginsSetEnabled,
  pluginsStatus,
  selectMigrationFiles,
  type InstalledPlugin,
  type PluginsStatus,
} from "./api";
import { useLang } from "./i18n";

export default function PluginPanel() {
  const { t } = useLang();
  // UI 重构：本面板已归入「插件」主分区，默认展开——避免进入分区后再点一次入口
  const [open, setOpen] = useState(true);
  const [status, setStatus] = useState<PluginsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [migBusy, setMigBusy] = useState<string | null>(null);
  const [migResults, setMigResults] = useState<Record<string, string[]>>({});
  // X49：用户自备 ekey（QMC STag 尾标变体；仅本地传递给插件进程，零网络）
  const [ekey, setEkey] = useState("");

  const load = useCallback(async () => {
    // 服务端形态：插件能力为桌面专属——**不发起请求**，由渲染层显示形态说明
    // （此前会请求失败并把 `MF-DESKTOP-ONLY` 渲染成红色 Error，观感如缺陷）
    if (!IS_DESKTOP) return;
    setError(null);
    try {
      const s = await pluginsStatus();
      setStatus(s);
      setSelected(s.enabled);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    if (open) void load();
  }, [open, load]);

  const toggle = (name: string) => {
    setDirty(true);
    setSelected((prev) =>
      prev.includes(name) ? prev.filter((n) => n !== name) : [...prev, name]
    );
  };

  // P6b.2：高风险插件（格式迁移类）ACK 确认——面板内展示风险提示后确认
  const acknowledge = async (name: string) => {
    const ok = window.confirm(t.plugin.ackConfirm(name));
    if (!ok) return;
    setError(null);
    try {
      await pluginsAcknowledge(name);
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  // P6b.4：格式迁移入口——选文件 → 逐个经插件迁移（产物与源同目录，源不动）
  // X49：requires_ekey 分流计数（按扩展名预估）+ ekey 透传 + 业务码友好映射
  const migrateFiles = async (p: InstalledPlugin) => {
    setError(null);
    try {
      const files = await selectMigrationFiles(p.extensions);
      if (files.length === 0) return;
      setMigBusy(p.name);
      const lines: string[] = [];
      // 分流计数（RFC-0002 §2.2）：静态表系 / 尾标系 / 待判定（无数字 mflac/mgg
      // 可能带 STag，归待判定——插件侧以最末 4 字节精确判定）
      const extOf = (f: string) => {
        const m = /\.([A-Za-z0-9]+)$/.exec(f);
        return (m?.[1] ?? "").toLowerCase();
      };
      const STATIC = new Set([
        "qmc0",
        "qmc3",
        "qmcmp3",
        "bkcmp3",
        "qmcflac",
        "qmflac",
        "bkcflac",
        "qmc2",
        "qmcogg",
      ]);
      const TAGGED = new Set(["mflac0", "mflac1", "mgg0", "mgg1", "mggl"]);
      let direct = 0;
      let needEkey = 0;
      let unknown = 0;
      for (const f of files) {
        const e = extOf(f);
        if (STATIC.has(e)) direct += 1;
        else if (TAGGED.has(e)) needEkey += 1;
        else unknown += 1;
      }
      lines.push(
        `${t.plugin.splitHead}: ${t.plugin.splitDirect(direct)} · ${t.plugin.splitEkey(needEkey)} · ${t.plugin.splitUnknown(unknown)}`
      );
      for (const f of files) {
        try {
          const r = await formatMigrate(p.name, f, undefined, ekey || undefined);
          lines.push(`✓ ${f} → ${r.outputPath}`);
        } catch (e) {
          const msg = String(e);
          if (msg.includes("QMC-EKEY-REQUIRED")) {
            lines.push(`✗ ${f}: ${t.plugin.ekeyRequired}`);
          } else if (msg.includes("QMC-EKEY-INVALID")) {
            lines.push(`✗ ${f}: ${t.plugin.ekeyInvalid}`);
          } else {
            lines.push(`✗ ${f}: ${msg}`);
          }
        }
      }
      setMigResults((prev) => ({ ...prev, [p.name]: lines }));
    } catch (e) {
      setError(String(e));
    } finally {
      setMigBusy(null);
    }
  };

  // X49：插件声明尾标系扩展名 → 显示 ekey 输入位
  const TAGGED_EXTS = new Set(["mflac", "mflac0", "mflac1", "mgg", "mgg0", "mgg1", "mggl"]);
  const needsEkeyInput = (p: InstalledPlugin) =>
    p.extensions.some((e) => TAGGED_EXTS.has(e.toLowerCase()));

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const r = await pluginsSetEnabled(selected);
      setSelected(r.enabled);
      setDirty(false);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  if (!open) {
    return (
      <button className="scan-toggle" onClick={() => setOpen(true)}>
        {t.plugin.toggle}
      </button>
    );
  }

  // 服务端形态（fnOS / 自建 server）：插件与 AI 能力为**桌面版专属**——
  // 显示形态说明卡（能力边界显式可见），而不是"请求失败"的红色错误条。
  if (!IS_DESKTOP) {
    return (
      <div className="panel">
        <div className="panel-head">
          <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.plugin.head}</h2>
        </div>
        <div className="state-hero" style={{ padding: "28px 16px" }}>
          <span className="state-ico dim" aria-hidden="true">
            <svg
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="3" y="3" width="7" height="7" rx="1.5" />
              <rect x="14" y="3" width="7" height="7" rx="1.5" />
              <rect x="3" y="14" width="7" height="7" rx="1.5" />
              <rect x="14" y="14" width="7" height="7" rx="1.5" />
            </svg>
          </span>
          <h2 style={{ fontSize: "var(--fs-md)" }}>{t.plugin.serverOnlyTitle}</h2>
          <p>{t.plugin.serverOnlyBody}</p>
          <ul className="state-facts" style={{ textAlign: "left" }}>
            <li>{t.plugin.serverOnlyServe}</li>
            <li>{t.plugin.serverOnlyLocal}</li>
          </ul>
        </div>
      </div>
    );
  }

  const runtime = status?.runtimeAvailable ?? false;
  const installed = status?.installed ?? [];
  const enabled = new Set(selected);
  const ackedList = new Set(status?.acked ?? []);

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>{t.plugin.head}</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          {t.plugin.collapse}
        </button>
      </div>

      {error && (
        <p className="plugin-error" role="alert">
          {error}
        </p>
      )}

      <p>
        {t.plugin.runtime}
        <b>{runtime ? t.plugin.runtimeOn : t.plugin.runtimeOff}</b>
        {status && <span className="plugin-note"> {t.plugin.runtimeNote(runtime)}</span>}
      </p>

      <p>
        {t.plugin.localSufficientA}
        <b>{t.plugin.localSufficientB}</b>
      </p>

      <h4>{t.plugin.installedHead}</h4>
      {installed.length === 0 ? (
        <p className="plugin-note">{t.plugin.noneInstalled}</p>
      ) : (
        <ul className="plugin-list">
          {installed.map((p) => {
            const needsAck = p.ackRequired && !ackedList.has(p.name);
            return (
              <li key={p.name}>
                <label>
                  <input
                    type="checkbox"
                    checked={enabled.has(p.name)}
                    onChange={() => toggle(p.name)}
                    disabled={!runtime || needsAck}
                  />{" "}
                  <b>{p.name}</b>
                  {t.plugin.meta(p.kind, p.apiVersion, p.network)}
                  {p.extensions.length > 0 && <>{t.plugin.extensions(p.extensions.join("/"))}</>}
                  {t.plugin.metaClose}
                  {needsAck && <b className="plugin-ack-warn">{t.plugin.needAck}</b>}
                </label>
                {needsAck && (
                  <button className="btn sm" onClick={() => void acknowledge(p.name)}>
                    {t.plugin.ackButton}
                  </button>
                )}
                {p.extensions.length > 0 && runtime && enabled.has(p.name) && (
                  <div className="plugin-migrate">
                    {needsEkeyInput(p) && (
                      <>
                        <label className="plugin-ekey-row">
                          <span className="plugin-note">{t.plugin.ekeyLabel}</span>
                          <input
                            className="plugin-ekey-input"
                            type="text"
                            value={ekey}
                            placeholder={t.plugin.ekeyPlaceholder}
                            onChange={(ev) => setEkey(ev.target.value)}
                            spellCheck={false}
                          />
                        </label>
                        <div className="plugin-note">{t.plugin.ekeyHint}</div>
                      </>
                    )}
                    <button
                      className="btn sm"
                      onClick={() => void migrateFiles(p)}
                      disabled={migBusy !== null}
                    >
                      {migBusy === p.name ? t.plugin.migrating : t.plugin.migrateButton}
                    </button>
                    {(migResults[p.name] ?? []).map((line, i) => (
                      <div key={i} className="plugin-note">
                        {line}
                      </div>
                    ))}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <div className="plugin-actions">
        <button
          className="btn sm"
          onClick={() => void save()}
          disabled={!dirty || saving || !runtime}
        >
          {saving ? t.plugin.saving : t.plugin.saveEnabled}
        </button>
        <button className="btn sm" onClick={() => void load()} disabled={saving}>
          {t.plugin.refresh}
        </button>
      </div>

      <h4>{t.plugin.dirsHead}</h4>
      <ul className="plugin-note">
        {(status?.pluginDirs ?? []).map((d) => (
          <li key={d}>{d}</li>
        ))}
      </ul>

      <p className="plugin-url">{t.plugin.learnMore}</p>
    </div>
  );
}
