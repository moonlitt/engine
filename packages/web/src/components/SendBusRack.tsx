import { useState } from 'react';
import type { SendBusView } from '@moonlitt/protocol';
import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { ParamSlider } from './ParamSlider';

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
      <div className="flex items-center gap-3 px-4 py-2 rounded border border-dashed border-daw-border bg-daw-surface/40 text-[11px] text-[#8a857b]">
        <span className="lcd-label text-[#7c776c]">送出母线</span>
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
    <section className="strip p-3">
      <div className="flex items-center gap-2 mb-2">
        <span className="lcd-label text-[#7c776c]">
          送出母线 · {buses.length}
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
            <BusCard key={bus.id} bus={bus} />
          ))}
        </div>
      )}
    </section>
  );
}

function BusCard({ bus }: { bus: SendBusView }) {
  const send = useSessionStore((s) => s.send);
  const setSendBusParamLocal = useProjectStore((s) => s.setSendBusParam);
  const [open, setOpen] = useState(false);

  return (
    <div className="rounded bg-daw-control/40 border border-daw-border text-[11px]">
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="w-3 text-[10px] text-[#888] hover:text-[#ccc]"
          title={open ? '收起参数' : '展开参数'}
        >{open ? '▾' : '▸'}</button>
        <span className="w-1.5 h-1.5 rounded-full bg-daw-accent shrink-0" />
        <span className="text-[10px] font-mono text-[#666]">#{bus.id}</span>
        <span className="text-[#e0e0e0] flex-1 truncate">{bus.name}</span>
        <span className="text-[9px] text-[#666] font-mono uppercase tracking-wider">
          {bus.effectType}
        </span>
      </div>
      {open && bus.params.length > 0 && (
        <div className="px-2.5 pb-2.5 pt-0.5 flex flex-col gap-1.5 border-t border-daw-border">
          {bus.params.map((p) => (
            <ParamSlider
              key={p.id}
              param={p}
              onChange={(value) => {
                setSendBusParamLocal(bus.id, p.id, value);
                send({ type: 'send_bus.set_param', busId: bus.id, paramId: p.id, value });
              }}
            />
          ))}
        </div>
      )}
      {open && bus.params.length === 0 && (
        <div className="px-2.5 pb-2 pt-0.5 text-[10px] text-[#666] border-t border-daw-border">
          此效果未暴露可调参数
        </div>
      )}
    </div>
  );
}
