import type { ParamMeta } from '@moonlitt/protocol';

/**
 * One labelled effect-parameter slider. Shared by the per-channel
 * insert cards and the send-bus rack so both render params identically.
 */
export function ParamSlider({ param, onChange }: { param: ParamMeta; onChange(v: number): void }) {
  const range = param.max - param.min;
  const step = param.stepCount > 0 ? range / param.stepCount : range / 1000;
  return (
    <div className="grid grid-cols-[7rem_1fr_3.5rem] items-center gap-2">
      <label className="text-[10px] text-[#aaa] truncate" title={param.name}>{param.name}</label>
      <input
        type="range"
        min={param.min} max={param.max} step={step} value={param.value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full accent-daw-accent"
      />
      <span className="text-[10px] text-[#888] text-right tabular-nums">{formatValue(param.value, param)}</span>
    </div>
  );
}

export function formatValue(value: number, param: ParamMeta): string {
  if (param.min === 0 && param.max === 1) return `${Math.round(value * 100)}%`;
  if (param.stepCount > 0 && param.stepCount <= 1) return value >= 0.5 ? '开' : '关';
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toFixed(0);
  if (abs >= 100) return value.toFixed(1);
  if (abs >= 10) return value.toFixed(2);
  return value.toFixed(3);
}
