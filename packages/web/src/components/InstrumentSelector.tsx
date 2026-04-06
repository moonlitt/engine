import { useState, useCallback } from 'react';

interface InstrumentSelectorProps {
  open: boolean;
  onLoad: (path: string) => void;
  onClose: () => void;
}

export function InstrumentSelector({ open, onLoad, onClose }: InstrumentSelectorProps) {
  const [path, setPath] = useState('');

  const handleLoad = useCallback(() => {
    const trimmed = path.trim();
    if (trimmed.length === 0) return;
    onLoad(trimmed);
    setPath('');
  }, [path, onLoad]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        handleLoad();
      } else if (e.key === 'Escape') {
        onClose();
      }
    },
    [handleLoad, onClose],
  );

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="bg-daw-panel border border-daw-border rounded-lg p-5 w-[400px] shadow-xl">
        <h3 className="text-sm font-medium text-[#e0e0e0] mb-3">Load Instrument</h3>
        <p className="text-xs text-[#888] mb-3">
          Enter the path to a .sf2 soundfont file.
        </p>
        <input
          type="text"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="/path/to/instrument.sf2"
          className="w-full bg-daw-control border border-daw-border rounded px-3 py-2 text-sm text-[#e0e0e0] placeholder-[#555] outline-none focus:border-daw-accent transition-colors"
          autoFocus
        />
        <div className="flex justify-end gap-2 mt-4">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-xs rounded bg-daw-control hover:bg-daw-border text-[#888] transition-colors"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleLoad}
            disabled={path.trim().length === 0}
            className="px-3 py-1.5 text-xs rounded bg-daw-accent hover:bg-daw-accent/80 text-white transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Load
          </button>
        </div>
      </div>
    </div>
  );
}
