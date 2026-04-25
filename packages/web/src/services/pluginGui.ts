/**
 * Open a plugin's native GUI window (Tauri + macOS only).
 *
 * Returns a user-friendly error message if the active transport doesn't
 * support GUI hosting (currently the WebSocket / browser build).
 */
import { getTransport } from './transport';

export type PluginGuiTarget =
  | { kind: 'default' }
  | { kind: 'override'; channel: number };

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isGuiSupported(): boolean {
  return typeof window !== 'undefined' && !!window.__TAURI_INTERNALS__;
}

export async function openPluginGui(target: PluginGuiTarget): Promise<string | null> {
  if (!isGuiSupported()) {
    return '插件原生界面仅在桌面 (Tauri) 模式可用';
  }
  try {
    const core = await import('@tauri-apps/api/core');
    await core.invoke('cmd_open_plugin_gui', { target });
    return null;
  } catch (err) {
    void getTransport(); // ensure transport exists; non-fatal
    return String(err);
  }
}
