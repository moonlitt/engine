import { useState } from 'react';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';

const BUS_EFFECT_TYPES: readonly { label: string; value: string }[] = [
  { label: 'Dattorro 混响', value: 'dattorro-reverb' },
  { label: 'Freeverb 混响', value: 'reverb' },
  { label: '延迟', value: 'delay' },
  { label: '合唱', value: 'chorus' },
  { label: '镶边', value: 'flanger' },
  { label: '相位器', value: 'phaser' },
];

/**
 * Send / aux bus rack. Sits above the channel list — each bus is one
 * effect that any channel can send into via the `Sends` row inside its
 * override body. Style mirrors a small mixer's aux section (Pro Tools'
 * "aux returns", Logic's "auxes").
 */
export function SendBusRack() {
  const buses = useProjectStore((s) => s.sendBuses);
  const send = useSessionStore((s) => s.send);
  const [adding, setAdding] = useState(false);

  if (buses.length === 0 && !adding) {
    return (
      <div className="flex items-center gap-3 px-4 py-2 rounded border border-dashed border-daw-border bg-daw-surface/40 text-[11px] text-[#888]">
        <span className="uppercase tracking-widest text-[#666]">送出母线</span>
        <span className="flex-1">尚未添加母线 — 添加一条混响或延迟，所有通道都可以送音过去</span>
        <button
          type="button"
          onClick={() => setAdding(true)}
          className="text-[10px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
        >
          + 添加母线
        </button>
      </div>
    );
  }

  return (
    <section className="bg-daw-panel border border-daw-border rounded-lg p-3">
      <div className="flex items-center gap-2 mb-2">
        <span className="text-[11px] uppercase tracking-widest text-[#888] font-semibold">
          送出母线（{buses.length}）
        </span>
        {!adding && (
          <button
            type="button"
            onClick={() => setAdding(true)}
            className="ml-auto text-[10px] px-2 py-0.5 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
          >+ 添加母线</button>
        )}
      </div>
      {adding && (
        <select
          autoFocus
          defaultValue=""
          onChange={(e) => {
            if (e.target.value) {
              send({ type: 'send_bus.add', effectType: e.target.value });
            }
            setAdding(false);
          }}
          onBlur={() => setAdding(false)}
          className="mb-2 bg-daw-control border border-daw-accent rounded px-2 py-1 text-xs text-[#e0e0e0] outline-none w-full"
        >
          <option value="" disabled>选择效果…</option>
          {BUS_EFFECT_TYPES.map((fx) => (
            <option key={fx.value} value={fx.value}>{fx.label}</option>
          ))}
        </select>
      )}
      {buses.length > 0 && (
        <div className="grid grid-cols-2 gap-2">
          {buses.map((bus) => (
            <div
              key={bus.id}
              className="flex items-center gap-2 px-2.5 py-1.5 rounded bg-daw-control/40 border border-daw-border text-[11px]"
            >
              <span className="w-1.5 h-1.5 rounded-full bg-daw-accent shrink-0" />
              <span className="text-[10px] font-mono text-[#666]">#{bus.id}</span>
              <span className="text-[#e0e0e0] flex-1 truncate">{bus.name}</span>
              <span className="text-[9px] text-[#666] font-mono uppercase tracking-wider">
                {bus.effectType}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
