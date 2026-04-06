interface PanKnobProps {
  value: number; // -1 (L) to +1 (R)
  onChange: (pan: number) => void;
}

function panLabel(v: number): string {
  if (Math.abs(v) < 0.05) return 'C';
  if (v < 0) return `L${Math.round(Math.abs(v) * 100)}`;
  return `R${Math.round(v * 100)}`;
}

export function PanKnob({ value, onChange }: PanKnobProps) {
  return (
    <div className="flex flex-col items-center gap-0.5">
      <input
        type="range"
        min={-100}
        max={100}
        value={Math.round(value * 100)}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
        onDoubleClick={() => onChange(0)}
        className="w-12 h-1.5 appearance-none bg-daw-bg rounded cursor-pointer
          [&::-webkit-slider-thumb]:appearance-none
          [&::-webkit-slider-thumb]:w-2.5
          [&::-webkit-slider-thumb]:h-2.5
          [&::-webkit-slider-thumb]:rounded-full
          [&::-webkit-slider-thumb]:bg-daw-accent
          [&::-webkit-slider-thumb]:border-none
          [&::-webkit-slider-thumb]:cursor-pointer"
      />
      <span className="text-[9px] text-[#666] select-none tabular-nums">
        {panLabel(value)}
      </span>
    </div>
  );
}
