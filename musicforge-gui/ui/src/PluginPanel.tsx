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
  formatMigrate,
  pluginsAcknowledge,
  pluginsSetEnabled,
  pluginsStatus,
  selectMigrationFiles,
  type InstalledPlugin,
  type PluginsStatus,
} from "./api";

export default function PluginPanel() {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<PluginsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [migBusy, setMigBusy] = useState<string | null>(null);
  const [migResults, setMigResults] = useState<Record<string, string[]>>({});

  const load = useCallback(async () => {
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
    const ok = window.confirm(
      `确认启用高风险插件「${name}」？\n\n` +
        "格式迁移类插件将在授权工作根内读写你合法持有的文件。\n" +
        "确认后写入 config.json plugins.acked（可随时删除该记录撤销）。"
    );
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
  const migrateFiles = async (p: InstalledPlugin) => {
    setError(null);
    try {
      const files = await selectMigrationFiles(p.extensions);
      if (files.length === 0) return;
      setMigBusy(p.name);
      const lines: string[] = [];
      for (const f of files) {
        try {
          const r = await formatMigrate(p.name, f);
          lines.push(`✓ ${f} → ${r.outputPath}`);
        } catch (e) {
          lines.push(`✗ ${f}: ${String(e)}`);
        }
      }
      setMigResults((prev) => ({ ...prev, [p.name]: lines }));
    } catch (e) {
      setError(String(e));
    } finally {
      setMigBusy(null);
    }
  };

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
        ▍AI 与插件
      </button>
    );
  }

  const runtime = status?.runtimeAvailable ?? false;
  const installed = status?.installed ?? [];
  const enabled = new Set(selected);
  const ackedList = new Set(status?.acked ?? []);

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>AI 与插件</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          收起
        </button>
      </div>

      {error && (
        <p className="plugin-error" role="alert">
          {error}
        </p>
      )}

      <p>
        插件运行时：<b>{runtime ? "已解锁" : "未安装"}</b>
        {status && (
          <span className="plugin-note">
            {" "}
            （构建{runtime ? "含" : "不含"} plugin-host；发行版默认含）
          </span>
        )}
      </p>

      <p>
        本地功能（扫描 / 清洗 / 去重 / 转换 / 整轨切分）
        <b>无需任何插件即可完整使用</b>。
      </p>

      <h4>已安装插件</h4>
      {installed.length === 0 ? (
        <p className="plugin-note">
          白名单目录内暂无插件。AI 识别、歌词/封面在线补全等能力由可选插件提供——
          插件默认禁用、独立分发，且永远不获得删除/移动/覆盖文件的权限。
        </p>
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
                  <b>{p.name}</b>（{p.kind}，api {p.apiVersion}，
                  {p.network ? "声明联网" : "离线"}
                  {p.extensions.length > 0 && <>，迁移 {p.extensions.join("/")}</>})
                  {needsAck && <b className="plugin-ack-warn">［需确认］</b>}
                </label>
                {needsAck && (
                  <button className="btn sm" onClick={() => void acknowledge(p.name)}>
                    确认启用（高风险）
                  </button>
                )}
                {p.extensions.length > 0 && runtime && enabled.has(p.name) && (
                  <div className="plugin-migrate">
                    <button
                      className="btn sm"
                      onClick={() => void migrateFiles(p)}
                      disabled={migBusy !== null}
                    >
                      {migBusy === p.name ? "迁移中…" : "选择文件并迁移"}
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
          {saving ? "保存中…" : "保存启用列表"}
        </button>
        <button className="btn sm" onClick={() => void load()} disabled={saving}>
          刷新
        </button>
      </div>

      <h4>插件目录（白名单）</h4>
      <ul className="plugin-note">
        {(status?.pluginDirs ?? []).map((d) => (
          <li key={d}>{d}</li>
        ))}
      </ul>

      <p className="plugin-url">
        了解插件：github.com/simenty/MusicForge/blob/master/PLUGIN_POLICY.md
      </p>
    </div>
  );
}
