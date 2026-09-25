// 排序切换条（P6.21）：分段按钮组，复用 .btn.sm 外观。
import type { TrackSortField } from "./lib/types";

export default function SortControl({
  value,
  onChange,
  fields,
  note = null,
}: {
  value: TrackSortField;
  onChange: (s: TrackSortField) => void;
  /** 可选排序键 + 文案（default 的文案各页不同：路径序 / 收藏时间 / 播放时间） */
  fields: { value: TrackSortField; label: string }[];
  /** P1-14：排序失效时的显式提示（null = 无）。失效必须可见——绝不能静默回退。 */
  note?: string | null;
}) {
  return (
    <div className="sort-ctrl" role="group" aria-label="排序">
      {fields.map((f) => (
        <button
          key={f.value}
          className={"btn sm" + (value === f.value ? " on" : "")}
          onClick={() => onChange(f.value)}
          aria-pressed={value === f.value}
        >
          {f.label}
        </button>
      ))}
      {note ? (
        <span className="sort-note" role="status">
          {note}
        </span>
      ) : null}
    </div>
  );
}
