/**
 * Open a plugin's native GUI window (Tauri + macOS only).
 *
 * Returns a user-friendly error message if the active transport doesn't
 * support GUI hosting (currently the WebSocket / browser build).
 */
import { getTransport, isTauriRuntime } from './transport';

export type PluginGuiTarget =
  | { kind: 'default' }
  | { kind: 'override'; channel: number };

export function isGuiSupported(): boolean {
  return isTauriRuntime();
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
