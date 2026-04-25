import { useCallback, useRef, useState } from 'react';
import { uploadMidiFile } from '../services/upload';

interface MidiBarProps {
  /** Currently-staged MIDI file display name, or null when nothing is loaded. */
  loadedName: string | null;
  /** Track count so we can summarise "N tracks across M channels". */
  trackCount: number;
  /** Whether the parent should render the "empty" hero zone instead of the slim bar. */
  hero: boolean;
}

/**
 * Single global MIDI upload area. Sits between the header and the track list.
 *
 * When nothing has been uploaded yet, the parent renders a hero variant
 * (full-width drop zone) instead — that's the empty-workspace CTA.
 *
 * Uploading goes through `/api/upload-midi` with trackId=0 (the server
 * ignores trackId in multi-track mode and creates / reuses tracks based on
 * the MIDI's channels).
 */
export function MidiBar({ loadedName, trackCount, hero }: MidiBarProps) {
  const [dragOver, setDragOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const upload = useCallback(async (file: File) => {
    if (!file.name.match(/\.midi?$/i)) {
      setError(`Not a MIDI file: ${file.name}`);
      return;
    }
    setError(null);
    setBusy(true);
    // The server picks the right track by parsing channels — trackId is
    // ignored in multi-track mode, but the upload form-data still needs it.
    const ok = await uploadMidiFile(file, 0);
    setBusy(false);
    if (!ok) setError('Upload failed (see server log)');
  }, []);

  const onDrop = useCallback(async (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) await upload(file);
  }, [upload]);

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) upload(file);
    e.target.value = '';
  };

  const dropProps = {
    onDragOver: (e: React.DragEvent) => { e.preventDefault(); setDragOver(true); },
    onDragLeave: () => setDragOver(false),
    onDrop,
    onClick: () => fileInputRef.current?.click(),
  };

  if (hero) {
    return (
      <div className="max-w-[820px] mx-auto px-6 pt-12">
        <div
          {...dropProps}
          className={`cursor-pointer rounded-xl border-2 border-dashed px-8 py-16 text-center transition-colors ${
            dragOver
              ? 'border-daw-accent bg-daw-accent/10'
              : 'border-daw-border hover:border-daw-accent/60 hover:bg-daw-control/30'
          }`}
        >
          <div className="text-4xl mb-3">📁</div>
          <div className="text-base text-[#e0e0e0] mb-1">
            {busy ? 'Uploading…' : 'Drop a .mid file to begin'}
          </div>
          <div className="text-xs text-[#888]">
            {busy ? '' : 'or click anywhere in this box to choose'}
          </div>
          <div className="text-[10px] text-[#555] mt-3">
            tracks will be created automatically — one per MIDI channel
          </div>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          accept=".mid,.midi"
          onChange={handleFileChange}
          className="hidden"
        />
        {error !== null && <div className="mt-3 text-xs text-red-400 text-center">{error}</div>}
      </div>
    );
  }

  return (
    <div className="max-w-[820px] mx-auto px-6 pt-4">
      <div
        {...dropProps}
        className={`cursor-pointer flex items-center gap-3 rounded border border-dashed px-4 py-2.5 text-xs transition-colors ${
          dragOver
            ? 'border-daw-accent bg-daw-accent/10'
            : 'border-daw-border hover:border-daw-accent/60 hover:bg-daw-control/30'
        }`}
      >
        <span>📁</span>
        <div className="flex-1 min-w-0">
          {loadedName ? (
            <>
              <span className="text-[#e0e0e0]">{loadedName}</span>
              <span className="text-[#666] ml-2">· {trackCount} track{trackCount === 1 ? '' : 's'}</span>
            </>
          ) : (
            <span className="text-[#aaa]">Drop a .mid file here, or click to choose</span>
          )}
        </div>
        <span className="text-[10px] text-daw-accent/80">
          {busy ? 'Uploading…' : loadedName ? 'Replace' : 'Choose…'}
        </span>
      </div>
      <input
        ref={fileInputRef}
        type="file"
        accept=".mid,.midi"
        onChange={handleFileChange}
        className="hidden"
      />
      {error !== null && <div className="mt-1 text-[11px] text-red-400">{error}</div>}
    </div>
  );
}
