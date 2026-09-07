// P6a 前置（X37）：「AI 与插件」占位态。
//
// 设计意图（方案 v2.7.2）：v0.7.0 上线时用户看到的是「解锁」而非「新增陌生入口」；
// 页面本身**零请求**——不加载任何远程资源，不违背零网络叙事。

import { useState } from "react";

export default function PluginPanel() {
  const [open, setOpen] = useState(false);

  if (!open) {
    return (
      <button className="scan-toggle" onClick={() => setOpen(true)}>
        ▍AI 与插件（未安装插件运行时）
      </button>
    );
  }

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>AI 与插件</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          收起
        </button>
      </div>
      <div className="plugin-placeholder">
        <p>当前版本未安装插件运行时。</p>
        <p>
          本地功能（扫描 / 清洗 / 去重 / 转换 / 整轨切分）
          <b>无需任何插件即可完整使用</b>。
        </p>
        <p className="plugin-note">
          AI 识别、歌词/封面在线补全、加密格式迁移等能力将由 v0.7.0 起的可选插件提供
          —— 插件默认禁用、独立分发，且永远不获得删除/移动/覆盖文件的权限。
        </p>
        <p className="plugin-url">
          了解插件：github.com/simenty/MusicForge/blob/master/PLUGIN_POLICY.md
        </p>
      </div>
    </div>
  );
}
