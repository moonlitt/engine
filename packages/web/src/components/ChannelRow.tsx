import { useCallback, useState } from 'react';
import type {
  ChannelOverrideState,
  InsertState,
  MidiChannelInfo,
  ParamMeta,
} from '@moonlitt/protocol';
import { useSessionStore } from '../stores/session';
import { useUiStore } from '../stores/ui';
import { useProjectStore } from '../stores/project';
import { channelDisplayName } from '../i18n/gm-programs';

interface ChannelRowProps {
  info: MidiChannelInfo;
  override: ChannelOverrideState | null;
}

const EFFECT_TYPES: readonly { label: string; value: string }[] = [
  { label: 'Dattorro 混响', value: 'dattorro-reverb' },
  { label: 'Freeverb 混响', value: 'reverb' },
  { label: '延迟', value: 'delay' },
  { label: '合唱', value: 'chorus' },
  { label: '镶边', value: 'flanger' },
  { label: '相位器', value: 'phaser' },
  { label: '颤音', value: 'tremolo' },
  { label: 'EQ 均衡器', value: 'eq' },
  { label: '压缩器', value: 'compressor' },
  { label: '限幅器', value: 'limiter' },
  { label: '门限', value: 'gate' },
  { label: '齿音消除', value: 'deesser' },
  { label: '多段压缩器', value: 'multiband-compressor' },
  { label: '饱和器', value: 'saturator' },
  { label: '位压缩 (Bitcrusher)', value: 'bitcrusher' },
  { label: '自动滤波', value: 'auto-filter' },
  { label: '变调', value: 'pitch-shifter' },
  { label: '立体声宽度', value: 'stereo-width' },
  { label: '增益', value: 'gain' },
];

export function ChannelRow({ info, override }: ChannelRowProps) {
  const send = useSessionStore((s) => s.send);
  const openPicker = useUiStore((s) => s.openInstrumentPicker);

  const inherited = override === null;
  const displayName = channelDisplayName(info.displayNumber, info.trackName, info.program);

  return (
    <section className="bg-daw-panel border border-daw-border rounded-lg overflow-hidden">
      {/* Header strip */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-daw-border">
        <div className={`w-8 text-center text-[10px] font-mono rounded px-1 py-0.5 ${
          inherited ? 'bg-daw-control text-[#aaa]' : 'bg-daw-accent/30 text-daw-accent'
        }`}>
          ch{info.displayNumber}
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm text-[#e0e0e0] truncate">{displayName}</div>
          {info.program !== undefined && info.trackName === undefined && (
            <div className="text-[10px] text-[#666]">GM 音色 #{info.program}</div>
          )}
        </div>

        {inherited ? (
          <button
            type="button"
            onClick={() => openPicker({ kind: 'override', channel: info.channel })}
            className="text-[11px] px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors"
            title="给这个通道单独指定一个音色（覆盖默认）"
          >
            单独指定音色…
          </button>
        ) : (
          <OverrideControls override={override} />
        )}
      </div>

      {/* Inherited body */}
      {inherited ? (
        <div className="px-3 py-1.5 text-[10px] text-[#666]">
          沿用默认音色
        </div>
      ) : (
        <OverrideBody channel={info.channel} override={override} send={send} />
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------

function OverrideControls({ override }: { override: ChannelOverrideState }) {
  const send = useSessionStore((s) => s.send);
  const openPicker = useUiStore((s) => s.openInstrumentPicker);
  const updateChannel = useProjectStore((s) => s.updateChannel);

  return (
    <div className="flex items-center gap-1.5">
      <button
        type="button"
        onClick={() => {
          const next = !override.muted;
          updateChannel(override.channel, { muted: next });
          send({ type: 'channel.set_mute', channel: override.channel, muted: next });
        }}
        className={`w-6 h-6 text-[10px] font-bold rounded transition-colors ${
          override.muted ? 'bg-red-500/80 text-white' : 'bg-daw-bg text-[#666] hover:text-white'
        }`}
        title="静音"
      >M</button>
      <button
        type="button"
        onClick={() => {
          const next = !override.solo;
          updateChannel(override.channel, { solo: next });
          send({ type: 'channel.set_solo', channel: override.channel, solo: next });
        }}
        className={`w-6 h-6 text-[10px] font-bold rounded transition-colors ${
          override.solo ? 'bg-yellow-500/80 text-black' : 'bg-daw-bg text-[#666] hover:text-white'
        }`}
        title="独奏"
      >S</button>
      <button
        type="button"
        onClick={() => openPicker({ kind: 'override', channel: override.channel })}
        className="text-[11px] px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
      >
        🎹 {override.instrumentName}
      </button>
      <button
        type="button"
        onClick={() => send({ type: 'channel.remove_override', channel: override.channel })}
        className="text-[11px] px-2 py-1 rounded text-[#888] hover:text-red-400 transition-colors"
        title="恢复默认音色"
      >× 恢复默认</button>
    </div>
  );
}

function OverrideBody({
  channel, override, send,
}: { channel: number; override: ChannelOverrideState; send: ReturnType<typeof useSessionStore.getState>['send'] }) {
  const [adding, setAdding] = useState(false);
  const updateChannel = useProjectStore((s) => s.updateChannel);
  const removeInsertLocal = useProjectStore((s) => s.removeInsert);
  const setInsertParamLocal = useProjectStore((s) => s.setInsertParam);

  const handleVolume = useCallback((db: number) => {
    updateChannel(channel, { volume: db });
    send({ type: 'channel.set_volume', channel, db });
  }, [channel, send, updateChannel]);

  return (
    <div className="px-3 py-2 flex flex-col gap-2">
      {/* Volume row */}
      <div className="flex items-center gap-3 text-[11px] text-[#888]">
        <span className="w-12 shrink-0">音量</span>
        <input
          type="range"
          min={-60} max={6} step={0.5}
          value={override.volume}
          onChange={(e) => handleVolume(parseFloat(e.target.value))}
          className="flex-1 accent-daw-accent"
        />
        <span className="w-12 text-right tabular-nums font-mono">{override.volume.toFixed(1)} dB</span>
      </div>

      {/* Effects */}
      <div className="border-t border-daw-border pt-2 flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <span className="text-[10px] uppercase tracking-widest text-[#888]">
            效果器（{override.inserts.length}）
          </span>
          {!adding && (
            <button
              type="button"
              onClick={() => setAdding(true)}
              className="text-[10px] px-2 py-0.5 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
            >+ 添加效果</button>
          )}
        </div>
        {adding && (
          <select
            autoFocus defaultValue=""
            onChange={(e) => {
              if (e.target.value) {
                send({ type: 'insert.add', channel, effectType: e.target.value });
              }
              setAdding(false);
            }}
            onBlur={() => setAdding(false)}
            className="bg-daw-control border border-daw-accent rounded px-2 py-1 text-xs text-[#e0e0e0] outline-none"
          >
            <option value="" disabled>选择效果…</option>
            {EFFECT_TYPES.map((fx) => (
              <option key={fx.value} value={fx.value}>{fx.label}</option>
            ))}
          </select>
        )}

        {override.inserts.map((insert) => (
          <InsertCard
            key={insert.id}
            channel={channel}
            insert={insert}
            onRemove={() => {
              removeInsertLocal(channel, insert.id);
              send({ type: 'insert.remove', channel, insertId: insert.id });
            }}
            onParam={(paramId, value) => {
              setInsertParamLocal(channel, insert.id, paramId, value);
              send({ type: 'insert.set_param', channel, insertId: insert.id, paramId, value });
            }}
          />
        ))}
      </div>
    </div>
  );
}

function InsertCard({
  insert, onRemove, onParam,
}: {
  channel: number;
  insert: InsertState;
  onRemove(): void;
  onParam(paramId: number, value: number): void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <div className="rounded border border-daw-border bg-daw-control/40">
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="w-3 text-[10px] text-[#888] hover:text-[#ccc]"
        >{open ? '▾' : '▸'}</button>
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 shrink-0" />
        <span className="text-xs text-[#e0e0e0] flex-1 truncate">{insert.name}</span>
        <button
          type="button"
          onClick={onRemove}
          className="text-[#666] hover:text-red-400 transition-colors px-1"
          title="移除"
        >×</button>
      </div>
      {open && insert.params.length > 0 && (
        <div className="px-2.5 pb-2.5 pt-0.5 flex flex-col gap-1.5 border-t border-daw-border">
          {insert.params.map((p) => (
            <ParamSlider key={p.id} param={p} onChange={(v) => onParam(p.id, v)} />
          ))}
        </div>
      )}
    </div>
  );
}

function ParamSlider({ param, onChange }: { param: ParamMeta; onChange(v: number): void }) {
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

function formatValue(value: number, param: ParamMeta): string {
  if (param.min === 0 && param.max === 1) return `${Math.round(value * 100)}%`;
  if (param.stepCount > 0 && param.stepCount <= 1) return value >= 0.5 ? '开' : '关';
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toFixed(0);
  if (abs >= 100) return value.toFixed(1);
  if (abs >= 10) return value.toFixed(2);
  return value.toFixed(3);
}
