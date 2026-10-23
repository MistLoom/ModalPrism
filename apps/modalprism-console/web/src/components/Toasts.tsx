import { useConsole } from "../lib/console";

export function Toasts() {
  const { toasts, dismiss } = useConsole();
  if (!toasts.length) {
    return null;
  }
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast is-${toast.tone}`}>
          <button
            type="button"
            style={{ display: "contents", textAlign: "left" }}
            onClick={() => dismiss(toast.id)}
            aria-label="关闭提示"
          >
            <span className="toast-message">{toast.message}</span>
            {toast.hint ? <span className="toast-hint">{toast.hint}</span> : null}
          </button>
        </div>
      ))}
    </div>
  );
}
