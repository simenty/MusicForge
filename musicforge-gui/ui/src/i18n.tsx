import {
  createContext,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { zh } from "./i18n/zh";
import { en } from "./i18n/en";

/**
 * P7 GUI i18n（ROADMAP P7：GUI i18n 中英）。
 *
 * 设计约束（对齐项目零依赖偏好）：
 * - 不引入 i18next 等运行时依赖——字典即普通对象，`t` 就是当前语言的那棵树；
 * - **类型安全**：`en` 的类型声明为 `typeof zh`——两语言形状漂移在编译期报错，
 *   绝不出现"切到英文变 undefined"这类静默断裂；
 * - 文案含插值的用**函数值**（`(n: number) => string`），不造模板引擎；
 * - 语言持久化 localStorage（`mf.lang`），默认跟随 `navigator.language`。
 */
export type Lang = "zh" | "en";

/** 字典树（zh 为形状源；en 必须同形）。 */
export type Dict = typeof zh;

const DICTS: Record<Lang, Dict> = { zh, en };

const LS_KEY = "mf.lang";

/** 初始语言：手动覆盖 > 跟随系统。 */
export function detectLang(): Lang {
  try {
    const saved = localStorage.getItem(LS_KEY);
    if (saved === "zh" || saved === "en") return saved;
  } catch {
    // localStorage 不可用（极端环境）→ 跟随系统，绝不因此崩溃
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

interface I18nContextValue {
  lang: Lang;
  setLang: (l: Lang) => void;
  /** 当前语言字典（用法：`const { t } = useLang(); t.app.addFiles`） */
  t: Dict;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detectLang);
  const value = useMemo<I18nContextValue>(
    () => ({
      lang,
      setLang: (l) => {
        try {
          localStorage.setItem(LS_KEY, l);
        } catch {
          // 持久化失败仅影响下次启动的默认值，当前会话切换照常生效
        }
        setLangState(l);
      },
      t: DICTS[lang],
    }),
    [lang]
  );
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useLang(): I18nContextValue {
  const v = useContext(I18nContext);
  if (!v) throw new Error("useLang must be used within I18nProvider");
  return v;
}
