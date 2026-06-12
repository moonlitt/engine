import { useToastStore } from '../stores/toasts';

/** Top-right toast rail. Click to dismiss early. */
export function Toasts() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed top-14 right-4 z-[60] flex flex-col gap-2 w-[340px]">
      {toasts.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => dismiss(t.id)}
          className={`strip-in text-left px-3.5 py-2.5 rounded-md border text-xs leading-relaxed shadow-strip ${
            t.kind === 'error'
              ? 'bg-[#2e1d1d] border-red-500/40 text-[#e8b8b0]'
              : 'bg-daw-panel border-daw-border text-[#d8d4cc]'
          }`}
          title="点击关闭"
        >
          {t.text}
        </button>
      ))}
    </div>
  );
}
