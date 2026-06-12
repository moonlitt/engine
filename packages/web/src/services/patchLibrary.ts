/**
 * STEAM patch-library access (Spectrasonics: Keyscape / Omnisphere /
 * Trilian). The backend scans the on-disk factory `.db` containers and
 * loads patches by state assembly — see
 * `crates/moonlitt-vst3/src/spectrasonics.rs`.
 */
import { isTauriRuntime } from './transport';

export type PatchTarget = { kind: 'default' } | { kind: 'override'; channel: number };

export interface LibraryPatchView {
  id: number;
  name: string;
  category: string;
  library: string;
}

export type ListResult =
  | { ok: true; patches: LibraryPatchView[] }
  | { ok: false; error: string };

export type LoadResult = { ok: true; patchName: string | null } | { ok: false; error: string };

/** Which instrument paths have a browsable STEAM library. */
export function hasPatchLibrary(instrumentPath: string | null): boolean {
  if (!instrumentPath || !isTauriRuntime()) return false;
  const stem = instrumentPath.split('/').pop() ?? '';
  return /^(keyscape|omnisphere|trilian)\.vst3$/i.test(stem);
}

export async function listLibraryPatches(target: PatchTarget): Promise<ListResult> {
  if (!isTauriRuntime()) {
    return { ok: false, error: '音色库浏览仅在桌面 (Tauri) 模式可用' };
  }
  try {
    const core = await import('@tauri-apps/api/core');
    const patches = await core.invoke<LibraryPatchView[]>('cmd_patch_library_list', { target });
    return { ok: true, patches };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function loadLibraryPatch(target: PatchTarget, patchId: number): Promise<LoadResult> {
  if (!isTauriRuntime()) {
    return { ok: false, error: '音色库浏览仅在桌面 (Tauri) 模式可用' };
  }
  try {
    const core = await import('@tauri-apps/api/core');
    const patchName = await core.invoke<string | null>('cmd_patch_library_load', {
      target,
      patchId,
    });
    return { ok: true, patchName };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
