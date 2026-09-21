// 列表文本筛选框（P6.25）：受控输入 + 一键清除。
//
// 为什么服务端过滤而不用前端 Array.filter：曲库页是**虚拟化**列表——
// 一次只拉取可视区间附近的若干页。若只在前端裁剪已加载的行，滚动到底
// 会按「未过滤的总数」继续请求，结果就是错位与空白占位。因此过滤条件
// 必须下推到 SQL（见 db.rs `track_filter_pred`），行数也走服务端计数。
export default function FilterInput({
  value,
  onChange,
  placeholder,
  clearLabel,
}: {
  value: string;
  onChange: (s: string) => void;
  placeholder: string;
  clearLabel: string;
}) {
  return (
    <div className="sort-ctrl" role="group" aria-label={placeholder}>
      <input
        className="cfg-input"
        type="search"
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      {value !== "" && (
        <button
          className="btn sm"
          onClick={() => onChange("")}
          aria-label={clearLabel}
          title={clearLabel}
        >
          ✕
        </button>
      )}
    </div>
  );
}
