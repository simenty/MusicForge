import { useCallback, useState } from "react";

/** 轻提示（3.6s 自动消失；同名消息互相不打断） */
export function useToast() {
  const [toast, setToast] = useState<string | null>(null);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    window.setTimeout(() => setToast((cur) => (cur === msg ? null : cur)), 3600);
  }, []);

  return { toast, setToast, showToast };
}
