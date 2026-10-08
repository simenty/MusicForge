// 风格码卡片测试（X15）。
//
// 重点验证 **I18N-7**：后端只回结构化数据 / 原始码，字段标签由前端按 UI 语言渲染
// ——英文界面下卡片不得出现中文（编译通过 ≠ 英文生效，故做运行时断言）。
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  styleCode: vi.fn(async () => null),
  trackGenre: vi.fn(async () => null),
}));

import { styleCode, trackGenre } from "./api";
import { I18nProvider } from "./i18n";
import StyleCodePanel from "./StyleCodePanel";
import type { StyleCodeDto } from "./lib/types";

const mockStyleCode = vi.mocked(styleCode);
const mockTrackGenre = vi.mocked(trackGenre);

/** 完整码块：`[Y23-S01-E01-C01-C02-V00]`（未配 codebook → labels 空） */
const FULL: StyleCodeDto = {
  year: 2023,
  style: "S01",
  mood: "E01",
  scenes: ["C01", "C02"],
  version: "V00",
  other: [],
  labels: {},
};

/** 配了 codebook：S01 → 流行，C01 → 学习（C02 未收录 → 仍需回退原始码） */
const WITH_BOOK: StyleCodeDto = {
  ...FULL,
  labels: { S01: "流行", C01: "学习" },
};

const hasCjk = (s: string) => /[一-鿿]/.test(s);

function renderPanel(path: string | null) {
  return render(
    <I18nProvider>
      <StyleCodePanel path={path} title="Sun" onClose={() => {}} />
    </I18nProvider>,
  );
}

/** 语言由 localStorage（`mf.lang`）决定——I18nProvider 用 detectLang() 初始化。 */
function setLang(lang: "zh" | "en") {
  try {
    localStorage.setItem("mf.lang", lang);
  } catch {
    /* 存储不可用则跟随系统，不影响断言意图 */
  }
}

describe("StyleCodePanel", () => {
  beforeEach(() => {
    mockStyleCode.mockReset();
    mockStyleCode.mockResolvedValue(null);
    mockTrackGenre.mockReset();
    mockTrackGenre.mockResolvedValue(null);
    setLang("zh");
  });

  it("无路径时直接显示「无码块」且不调后端", async () => {
    renderPanel(null);
    expect(await screen.findByText(/无风格码块/)).toBeTruthy();
    expect(mockStyleCode).not.toHaveBeenCalled();
  });

  it("渲染完整码块的各字段（中文）", async () => {
    mockStyleCode.mockResolvedValue(FULL);
    renderPanel("/lib/[Y23-S01-E01-C01-C02-V00] 晴天.flac");
    await waitFor(() => expect(screen.getByText("2023")).toBeTruthy());
    expect(screen.getByText("年份")).toBeTruthy();
    expect(screen.getByText("风格")).toBeTruthy();
    expect(screen.getByText("情绪")).toBeTruthy();
    expect(screen.getByText("场景")).toBeTruthy();
    // 多场景合并为一行
    expect(screen.getByText("C01 · C02")).toBeTruthy();
    expect(screen.getByText("版本")).toBeTruthy();
  });

  // I18N-7：英文界面不得弹中文标签
  it("英文界面下标签为英文且不含中文", async () => {
    setLang("en");
    mockStyleCode.mockResolvedValue(FULL);
    renderPanel("/lib/[Y23-S01-E01-C01-C02-V00] sun.flac");
    await waitFor(() => expect(screen.getByText("2023")).toBeTruthy());
    expect(screen.getByText("Year")).toBeTruthy();
    expect(screen.getByText("Style")).toBeTruthy();
    expect(screen.getByText("Mood")).toBeTruthy();
    expect(screen.getByText("Scene")).toBeTruthy();
    expect(screen.getByText("Version")).toBeTruthy();
    // 整个卡片正文无中文（标题含 ASCII 曲名，故取 modal 文本）
    const modal = document.querySelector(".sc-modal");
    expect(modal).toBeTruthy();
    expect(hasCjk(modal?.textContent ?? "")).toBe(false);
  });

  it("后端返回 null（无前导码块）时按说明文案展示，不报错", async () => {
    mockStyleCode.mockResolvedValue(null);
    renderPanel("/lib/晴天.flac");
    expect(await screen.findByText(/无风格码块/)).toBeTruthy();
  });

  it("解析失败时展示错误而非静默空卡片", async () => {
    mockStyleCode.mockRejectedValue(new Error("boom"));
    renderPanel("/lib/[Y23] a.flac");
    expect(await screen.findByText("Error: boom")).toBeTruthy();
  });

  // ---- codebook：查到用码名，查不到回退原始码（绝不编造）----
  it("配 codebook 时显示码名，未收录的码回退原始码", async () => {
    mockStyleCode.mockResolvedValue(WITH_BOOK);
    render(
      <I18nProvider>
        <StyleCodePanel
          path="/lib/[Y23-S01-E01-C01-C02-V00] 晴天.flac"
          title="晴天"
          codebookPath="/cfg/codebook.json"
          onClose={() => {}}
        />
      </I18nProvider>,
    );
    await waitFor(() => expect(screen.getByText("流行")).toBeTruthy());
    // 场景行：C01 → 学习（译名），C02 未收录 → 回退原始码，绝不编造
    expect(screen.getByText("学习 · C02")).toBeTruthy();
    // 已配置 codebook 时不再显示「未配置」提示
    expect(screen.queryByText(/未配置 codebook/)).toBeNull();
    // 且把 codebook 路径传给了后端
    expect(mockStyleCode).toHaveBeenCalledWith(
      "/lib/[Y23-S01-E01-C01-C02-V00] 晴天.flac",
      "/cfg/codebook.json",
    );
  });

  // ---- 文件现有 genre（写回前先看现状）----
  it("展示文件现有 genre；无标签时显式说「（无）」而非留空", async () => {
    mockStyleCode.mockResolvedValue(FULL);
    mockTrackGenre.mockResolvedValue("流行");
    renderPanel("/lib/[Y23-S01] a.flac");
    await waitFor(() => expect(screen.getByText("文件 genre")).toBeTruthy());
    expect(screen.getByText("流行")).toBeTruthy();

    // 无标签分支
    mockTrackGenre.mockResolvedValue(null);
    renderPanel("/lib/[Y23-S01] b.flac");
    await waitFor(() => expect(screen.getByText("（无）")).toBeTruthy());
  });

  it("未配 codebook 时给出「显示为原始码」说明", async () => {
    mockStyleCode.mockResolvedValue(FULL);
    renderPanel("/lib/[Y23-S01] a.flac");
    await waitFor(() => expect(screen.getByText("S01")).toBeTruthy());
    expect(screen.getByText(/未配置 codebook/)).toBeTruthy();
  });
});
