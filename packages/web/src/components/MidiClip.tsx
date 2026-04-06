interface MidiClipProps {
  clip: { id: number; name: string; startBar: number; lengthBars: number };
  color: string;
  pixelsPerBar: number;
}

export function MidiClip({ clip, color, pixelsPerBar }: MidiClipProps) {
  const left = clip.startBar * pixelsPerBar;
  const width = clip.lengthBars * pixelsPerBar;

  return (
    <div
      className="absolute top-1 bottom-1 rounded-sm border border-white/10 overflow-hidden
        flex items-end px-1 pb-0.5"
      style={{
        left: `${left}px`,
        width: `${width}px`,
        backgroundColor: `${color}40`,
        borderColor: `${color}80`,
      }}
    >
      <span className="text-[9px] text-white/70 truncate select-none">
        {clip.name}
      </span>
    </div>
  );
}
