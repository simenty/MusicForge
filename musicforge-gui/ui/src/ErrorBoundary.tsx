/**
 * 分区级错误边界（P0-2）。
 *
 * 目的：任一分区的渲染异常不该带走整个应用（此前无边界 → 整页白屏，
 * 在 fnOS 形态表现为「应用初始化异常」弹窗，无从判断是哪个模块出错）。
 *
 * 实现约束：React 错误边界必须是 class 组件，因此文案由外部（App）以 props
 * 传入（i18n 走 hook，class 内不可用），保持与项目 i18n 体系一致。
 */
import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  title: string;
  hint: string;
  retry: string;
  children: ReactNode;
}

interface State {
  err: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { err: null };

  static getDerivedStateFromError(err: Error): State {
    return { err };
  }

  componentDidCatch(err: Error, info: ErrorInfo): void {
    // 最小可观测性：渲染异常落到控制台（后续接 tracing/上报时可复用此点）
    // eslint-disable-next-line no-console
    console.error("[musicforge] render error:", err, info.componentStack);
  }

  render(): ReactNode {
    const { err } = this.state;
    if (!err) return this.props.children;
    return (
      <div className="panel" role="alert">
        <div className="panel-head">
          <h2>{this.props.title}</h2>
        </div>
        <p className="plugin-note">{this.props.hint}</p>
        <pre className="plugin-error">{String(err.message || err)}</pre>
        <div>
          <button className="btn sm" onClick={() => this.setState({ err: null })}>
            {this.props.retry}
          </button>
        </div>
      </div>
    );
  }
}
