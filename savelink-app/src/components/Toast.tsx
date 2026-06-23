import { createContext, useContext, useState, useCallback, type ReactNode } from "react";
import { Icon } from "../lib/icons";

type ToastKind = "ok" | "warn" | "err";
interface ToastItem { id: number; msg: string; kind: ToastKind; }

const ToastCtx = createContext<(msg: string, kind?: ToastKind) => void>(() => {});

export function useToast() {
  return useContext(ToastCtx);
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const push = useCallback((msg: string, kind: ToastKind = "ok") => {
    const id = Date.now() + Math.random();
    setItems((prev) => [...prev, { id, msg, kind }]);
    setTimeout(() => setItems((prev) => prev.filter((t) => t.id !== id)), 2800);
  }, []);

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toasts">
        {items.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.kind === "ok" ? <Icon.CheckCircle /> : <Icon.Alert />}
            <span>{t.msg}</span>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}
