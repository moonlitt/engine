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

/** Result of opening a plug-in GUI: either the window's label (for
 *  later `saveOpenPluginState` calls) or a user-friendly error. */
export type OpenPluginGuiResult =
  | { ok: true; label: string }
  | { ok: false; error: string };

export function isGuiSupported(): boolean {
  return isTauriRuntime();
}

export async function openPluginGui(target: PluginGuiTarget): Promise<OpenPluginGuiResult> {
  if (!isGuiSupported()) {
    return { ok: false, error: '插件原生界面仅在桌面 (Tauri) 模式可用' };
  }
  try {
    const core = await import('@tauri-apps/api/core');
    const label = await core.invoke<string>('cmd_open_plugin_gui', { target });
    return { ok: true, label };
  } catch (err) {
    void getTransport();
    return { ok: false, error: String(err) };
  }
}

/**
 * Capture the current plug-in state of an open GUI window and write it
 * to a binary file. Used for sample-based plug-ins (Keyscape,
 * Omnisphere) whose patch picker only exists in their private UI -- the
 * resulting file can be replayed headlessly via `moonlitt midi --state`.
 *
 * Pops a save dialog so the user picks the destination. Returns the
 * number of bytes written, or an error string.
 */
export async function saveOpenPluginState(
  label: string,
  defaultName: string,
): Promise<{ ok: true; bytes: number; path: string } | { ok: false; error: string }> {
  if (!isGuiSupported()) {
    return { ok: false, error: '仅 Tauri 桌面端可保存插件状态' };
  }
  try {
    const dialog = await import('@tauri-apps/plugin-dialog');
    const core = await import('@tauri-apps/api/core');
    const path = await dialog.save({
      title: '保存插件状态',
      defaultPath: defaultName,
      filters: [{ name: '插件状态', extensions: ['mlstate', 'bin'] }],
    });
    if (!path) {
      return { ok: false, error: '已取消' };
    }
    const bytes = await core.invoke<number>('cmd_save_plugin_state', { label, path });
    return { ok: true, bytes, path };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
