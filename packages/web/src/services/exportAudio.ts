import { invoke } from '@tauri-apps/api/core';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';

export interface RenderResult {
  path: string;
  durationSecs: number;
  peak: number;
}

/**
 * Ask for a destination and bounce the current project to a stereo
 * float-32 WAV. The render runs on a fresh engine instance in the
 * backend, so live playback keeps going. Resolves null when the user
 * cancels the dialog.
 */
export async function exportWav(suggestedName: string): Promise<RenderResult | null> {
  const path = await saveDialog({
    title: '导出 WAV',
    defaultPath: suggestedName,
    filters: [{ name: 'WAV 音频', extensions: ['wav'] }],
  });
  if (!path || typeof path !== 'string') return null;
  return invoke<RenderResult>('cmd_render_to_wav', { path });
}
