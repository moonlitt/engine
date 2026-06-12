import { create } from 'zustand';

/**
 * Global toast rail — surfaces command failures and notable automatic
 * actions (auto-picked default instrument, plug-in quarantines). The
 * alternative was console.error, which users never see.
 */
export interface Toast {
  id: number;
  kind: 'info' | 'error';
  text: string;
}

interface ToastStore {
  toasts: Toast[];
  push(kind: Toast['kind'], text: string): void;
  dismiss(id: number): void;
}

let nextId = 1;

export const useToastStore = create<ToastStore>((set) => ({
  toasts: [],
  push(kind, text) {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { id, kind, text }] }));
    const ttl = kind === 'error' ? 9000 : 5500;
    setTimeout(() => {
      set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    }, ttl);
  },
  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));

export function toastError(text: string) {
  useToastStore.getState().push('error', text);
}

export function toastInfo(text: string) {
  useToastStore.getState().push('info', text);
}
