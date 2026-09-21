// 排序切换条（P6.21）：分段按钮组，复用 .btn.sm 外观。
import type { TrackSortField } from "./lib/types";

export default function SortControl({
  value,
  onChange,
  fields,
}: {
  value: TrackSortField;
  onChange: (s: TrackSortField) => void;
  /** 可选排序键 + 文案（default 的文案各页不同：路径序 / 收藏时间 / 播放时间） */
  fields: { value: TrackSortField; label: string }[];
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
    </div>
  );
}
