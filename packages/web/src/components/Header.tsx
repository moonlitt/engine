interface HeaderProps {
  connected: boolean;
  playing: boolean;
  position: number;
  bpm: number;
  onPlay(): void;
  onStop(): void;
}

function formatBarsBeats(ticks: number): string {
  const tpq = 480;
  const beatsPerBar = 4;
  const ticksPerBar = tpq * beatsPerBar;
  const bar = Math.floor(ticks / ticksPerBar) + 1;
  const beat = Math.floor((ticks % ticksPerBar) / tpq) + 1;
  return `${bar}.${beat}`;
}

export function Header({ connected, playing, position, bpm, onPlay, onStop }: HeaderProps) {
  return (
    <div className="sticky top-0 z-10 bg-daw-panel border-b border-daw-border">
      <div className="max-w-[820px] mx-auto px-6 py-3 flex items-center gap-4">
        <div className="flex items-center gap-2">
          <span className="text-base font-semibold tracking-wide">
            moonlitt <span className="text-[#666] font-normal">player</span>
          </span>
          <div className="flex items-center gap-1.5 ml-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'}`} />
            <span className="text-[10px] text-[#888]">{connected ? 'connected' : 'offline'}</span>
          </div>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            onClick={onPlay}
            disabled={!connected}
            className={`px-4 py-1.5 rounded text-sm font-semibold transition-colors disabled:opacity-30 ${
              playing ? 'bg-daw-accent text-white' : 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
            }`}
            title="Toggle playback (Space)"
          >
            {playing ? '❚❚ Pause' : '▶ Play'}
          </button>
          <button
            type="button"
            onClick={onStop}
            disabled={!connected}
            className="px-3 py-1.5 rounded bg-daw-control hover:bg-daw-border text-[#e0e0e0] text-sm transition-colors disabled:opacity-30"
          >
            ■ Stop
          </button>
          <div className="ml-3 flex items-center gap-3 font-mono text-xs text-[#aaa]">
            <span><span className="text-[9px] text-[#666] mr-1">POS</span>{formatBarsBeats(position)}</span>
            <span><span className="text-[9px] text-[#666] mr-1">BPM</span>{bpm.toFixed(1)}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
