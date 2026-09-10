/**
 * 内联 SVG 图标（1.5px 描边 · currentColor · 16px 基准）。
 *
 * 约束：应用**离线运行**且不引第三方图标库（依赖面最小），
 * 因此图标一律内联为 SVG 组件——随 currentColor 自动适配主题与状态色。
 */

interface IconProps {
  size?: number;
  className?: string;
}

function svg(path: React.ReactNode, size: number, className?: string) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {path}
    </svg>
  );
}

/** 转换（主流程入口） */
export function IconConvert({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M4 7h9l-2.5-2.5" />
      <path d="M20 17h-9l2.5 2.5" />
    </>,
    size,
    className
  );
}

/** 曲库治理 */
export function IconLibrary({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H9l1.5 2h8A1.5 1.5 0 0 1 20 7.5v10A1.5 1.5 0 0 1 18.5 19h-13A1.5 1.5 0 0 1 4 17.5Z" />
    </>,
    size,
    className
  );
}

/** 插件 */
export function IconPlugin({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M10 4.5v3M14 16.5v3" />
      <path d="M6.5 7.5h11a2 2 0 0 1 2 2v1a3 3 0 0 0 0 6v1a2 2 0 0 1-2 2h-11a2 2 0 0 1-2-2v-1a3 3 0 0 0 0-6v-1a2 2 0 0 1 2-2Z" />
    </>,
    size,
    className
  );
}

/** 设置 */
export function IconSettings({ size = 16, className }: IconProps) {
  return svg(
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3v2.5M12 18.5V21M4.2 7.5l2.2 1.3M17.6 15.2l2.2 1.3M4.2 16.5l2.2-1.3M17.6 8.8l2.2-1.3" />
    </>,
    size,
    className
  );
}

export function IconPlus({ size = 16, className }: IconProps) {
  return svg(<path d="M12 5v14M5 12h14" />, size, className);
}

export function IconFolder({ size = 16, className }: IconProps) {
  return svg(
    <path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h3.6l1.8 2.2H19a1.5 1.5 0 0 1 1.5 1.5v7.3A1.5 1.5 0 0 1 19 18.5H5A1.5 1.5 0 0 1 3.5 17Z" />,
    size,
    className
  );
}

export function IconTrash({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M4.5 7h15M9.5 7V5.5A1 1 0 0 1 10.5 4.5h3a1 1 0 0 1 1 1V7" />
      <path d="M6.5 7l.8 11a1.5 1.5 0 0 0 1.5 1.4h6.4a1.5 1.5 0 0 0 1.5-1.4l.8-11" />
    </>,
    size,
    className
  );
}

export function IconPlay({ size = 16, className }: IconProps) {
  return svg(<path d="M8 5.5l10 6.5-10 6.5Z" />, size, className);
}

export function IconStop({ size = 16, className }: IconProps) {
  return svg(<rect x="7" y="7" width="10" height="10" rx="1.5" />, size, className);
}

export function IconPlan({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M14 3.5H6.5a1.5 1.5 0 0 0-1.5 1.5v14a1.5 1.5 0 0 0 1.5 1.5h11a1.5 1.5 0 0 0 1.5-1.5V8.5Z" />
      <path d="M14 3.5v5h5M9 13h6M9 16.5h4" />
    </>,
    size,
    className
  );
}

/** 曲库扫描 */
export function IconScan({ size = 16, className }: IconProps) {
  return svg(
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M15.8 15.8L20 20" />
    </>,
    size,
    className
  );
}

/** 重复去重（两份重叠） */
export function IconCopy({ size = 16, className }: IconProps) {
  return svg(
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M15 6.5V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h.5" />
    </>,
    size,
    className
  );
}

export function IconDownload({ size = 16, className }: IconProps) {
  return svg(
    <>
      <path d="M12 4v10M8 10.5l4 4 4-4" />
      <path d="M5 19h14" />
    </>,
    size,
    className
  );
}
