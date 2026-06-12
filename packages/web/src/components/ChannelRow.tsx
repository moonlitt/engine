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
import { GM_PROGRAM_ZH, channelDisplayName } from '../i18n/gm-programs';
import { isGuiSupported, openPluginGui } from '../services/pluginGui';
import { ParamSlider } from './ParamSlider';
import { Meter } from './Meter';

interface ChannelRowProps {
  info: MidiChannelInfo;
  override: ChannelOverrideState | null;
  defaultInstrumentPath: string | null;
}

// 8 muted-saturation DAW colours, in cycle order. Stays away from
// "AI vibe" violet — picks distinguishable hues that read well on the
// neutral-warm-grey background. Bitwig/Ableton-flavoured.
const TRACK_COLORS: readonly string[] = [
  '#c14d4d', // red
  '#cf8a3c', // orange
  '#c9b340', // yellow
  '#5c9a5c', // green
  '#4a9090', // teal
  '#5a9ad4', // blue (same as accent)
  '#9aa0a6', // silver (Bitwig-style neutral tint — no violet in this app)
  '#b8688f', // pink
];

function nextTrackColor(current: string | null | undefined): string | null {
  if (!current) return TRACK_COLORS[0];
  const idx = TRACK_COLORS.indexOf(current);
  if (idx < 0) return TRACK_COLORS[0];
  if (idx === TRACK_COLORS.length - 1) return null; // wrap back to "no color"
  return TRACK_COLORS[idx + 1];
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

export function ChannelRow({ info, override, defaultInstrumentPath }: ChannelRowProps) {
  const send = useSessionStore((s) => s.send);
  const openPicker = useUiStore((s) => s.openInstrumentPicker);

  const inherited = override === null;
  const displayName = channelDisplayName(info.displayNumber, info.trackName, info.program);

  // What instrument actually plays this channel right now?
  const activeInstrument = inherited
    ? defaultInstrumentPath?.split('/').pop() ?? null
    : override.instrumentName;

  const channelColor = override?.color ?? null;

  return (
    <section className="strip overflow-hidden flex">
      {/* Vertical colour stripe — Logic Pro-style track tint. Click to
          cycle through the palette; null means no tint. */}
      {!inherited && (
        <button
          type="button"
          onClick={() =>
            send({ type: 'channel.set_color', channel: info.channel, color: nextTrackColor(channelColor) })
          }
          className="w-1.5 shrink-0 transition-colors hover:opacity-80"
          style={{ backgroundColor: channelColor ?? '#2c2c2c' }}
          title="点击切换通道色"
          aria-label="切换通道色"
        />
      )}
    <div className="flex-1 min-w-0">
      {/* Header strip — channel-strip aesthetic: prominent ch# tile,
          tight padding, status colors only when an override is active. */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-daw-border bg-daw-surface/40">
        <div className={`w-10 h-8 flex flex-col items-center justify-center text-center font-mono rounded border ${
          inherited
            ? 'bg-daw-control border-daw-border text-[#9a9a9a]'
            : 'bg-daw-accent/20 border-daw-accent/60 text-daw-accent'
        }`}>
          <span className="text-[7px] uppercase tracking-wider leading-none opacity-70">ch</span>
          <span className="text-[11px] font-semibold leading-none mt-0.5">{info.displayNumber}</span>
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm text-[#e0e0e0] truncate">{displayName}</div>
          <div className="text-[10px] text-[#888] truncate">
            {inherited ? '沿用默认音色' : '单独指定音色'}
            {activeInstrument && (
              <span className="text-[#bbb] ml-1.5">· {activeInstrument}</span>
            )}
            {info.program !== undefined && info.trackName === undefined && (
              <span className="text-[#666] ml-1.5">· MIDI 默认 #{info.program} {GM_PROGRAM_ZH[info.program] ?? ''}</span>
            )}
          </div>
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
          <>
            {/* Live peak meter — only meaningful when an override owns
                its own mixer track; inherited channels mix into master. */}
            <Meter channel={info.channel} width={64} height={4} />
            <OverrideControls override={override} />
          </>
        )}
      </div>

      {/* Body — preset picker is universal; volume/effects only on overrides */}
      <ChannelBody info={info} override={override} send={send} />
    </div>
    </section>
  );
}

// ---------------------------------------------------------------------------

function ChannelBody({
  info, override, send,
}: {
  info: MidiChannelInfo;
  override: ChannelOverrideState | null;
  send: ReturnType<typeof useSessionStore.getState>['send'];
}) {
  const isOverride = override !== null;
  return (
    <div className="px-3 py-2 flex flex-col gap-2">
      <PresetPicker
        channel={info.channel}
        midiProgram={info.program}
        onPick={(program) => send({ type: 'channel.set_program', channel: info.channel, program })}
      />

      {isOverride && (
        <>
          <VolumeRow channel={info.channel} db={override.volume} />
          <PanRow channel={info.channel} pan={override.pan} />
          <SendsBlock channel={info.channel} sendLevels={override.sendLevels} />
          <EffectsBlock channel={info.channel} inserts={override.inserts} />
        </>
      )}
    </div>
  );
}

function SendsBlock({ channel, sendLevels }: { channel: number; sendLevels: number[] }) {
  const buses = useProjectStore((s) => s.sendBuses);
  const send = useSessionStore((s) => s.send);
  if (buses.length === 0) return null;
  return (
    <div className="border-t border-daw-border pt-2 flex flex-col gap-1">
      <div className="text-[10px] uppercase tracking-widest text-[#888]">
        送出 ({buses.length})
      </div>
      {buses.map((bus) => {
        const lvl = sendLevels[bus.id] ?? 0;
        return (
          <div key={bus.id} className="flex items-center gap-3 text-[11px] text-[#888]">
            <span className="w-12 shrink-0 truncate" title={bus.name}>
              <span className="text-[#666] mr-0.5">→</span>
              {bus.name.slice(0, 6)}
            </span>
            <input
              type="range"
              min={0}
              max={1.5}
              step={0.01}
              value={lvl}
              onChange={(e) =>
                send({
                  type: 'channel.set_send_level',
                  channel,
                  busId: bus.id,
                  level: parseFloat(e.target.value),
                })
              }
              onDoubleClick={() =>
                send({ type: 'channel.set_send_level', channel, busId: bus.id, level: 0 })
              }
              title="双击归零"
              className="flex-1 accent-daw-accent"
            />
            <span className="w-12 text-right tabular-nums font-mono">
              {lvl < 0.005 ? '·' : `${Math.round(lvl * 100)}%`}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function PanRow({ channel, pan }: { channel: number; pan: number }) {
  const send = useSessionStore((s) => s.send);
  const updateChannel = useProjectStore((s) => s.updateChannel);
  const apply = (v: number) => {
    updateChannel(channel, { pan: v });
    send({ type: 'channel.set_pan', channel, pan: v });
  };
  return (
    <div className="flex items-center gap-3 text-[11px] text-[#888]">
      <span className="w-12 shrink-0">声像</span>
      <span className="text-[10px] text-[#666] w-3 text-right">L</span>
      <input
        type="range"
        min={-1}
        max={1}
        step={0.02}
        value={pan}
        onChange={(e) => apply(parseFloat(e.target.value))}
        onDoubleClick={() => apply(0)}
        title="双击居中"
        className="flex-1 accent-daw-accent"
      />
      <span className="text-[10px] text-[#666] w-3">R</span>
      <span className="w-12 text-right tabular-nums font-mono">{formatPan(pan)}</span>
    </div>
  );
}

function formatPan(pan: number): string {
  if (Math.abs(pan) < 0.005) return 'C';
  const n = Math.round(pan * 100);
  return n < 0 ? `L${-n}` : `R${n}`;
}

function PresetPicker({
  channel, midiProgram, onPick,
}: {
  channel: number;
  midiProgram: number | undefined;
  onPick(program: number): void;
}) {
  const [value, setValue] = useState<string>('');
  // Channel 10 (display) is GM percussion — preset selection isn't really
  // meaningful; show a hint instead.
  const isDrumChannel = channel === 9;
  return (
    <div className="flex items-center gap-2 text-[11px]">
      <span className="text-[#888] w-12 shrink-0">音色</span>
      {isDrumChannel ? (
        <span className="text-[#aaa]">鼓组（GM 通道 10 固定为打击乐）</span>
      ) : (
        <>
          <select
            value={value}
            onChange={(e) => {
              const n = parseInt(e.target.value, 10);
              if (!Number.isNaN(n)) {
                onPick(n);
                setValue(String(n));
              }
            }}
            className="flex-1 bg-daw-control border border-daw-border rounded px-2 py-1 text-xs text-[#e0e0e0] outline-none focus:border-daw-accent"
            title="切换 GM 音色 — 注意 MIDI 文件本身的 Program Change 事件在播放过程中可能再次覆盖"
          >
            <option value="" disabled>
              {midiProgram !== undefined
                ? `沿用 MIDI 默认: #${midiProgram} ${GM_PROGRAM_ZH[midiProgram] ?? ''}`
                : '选择 GM 音色…'}
            </option>
            {GM_PROGRAM_ZH.map((name, i) => (
              <option key={i} value={i}>
                #{i} {name}
              </option>
            ))}
          </select>
          <span className="text-[10px] text-[#666] shrink-0" title="MIDI 文件里的 PC 事件可能在播放中再次覆盖">
            ⓘ
          </span>
        </>
      )}
    </div>
  );
}

function VolumeRow({ channel, db }: { channel: number; db: number }) {
  const send = useSessionStore((s) => s.send);
  const updateChannel = useProjectStore((s) => s.updateChannel);
  return (
    <div className="flex items-center gap-3 text-[11px] text-[#888]">
      <span className="w-12 shrink-0">音量</span>
      <input
        type="range"
        min={-60} max={6} step={0.5}
        value={db}
        onChange={(e) => {
          const v = parseFloat(e.target.value);
          updateChannel(channel, { volume: v });
          send({ type: 'channel.set_volume', channel, db: v });
        }}
        className="flex-1 accent-daw-accent"
      />
      <span className="w-12 text-right tabular-nums font-mono">{db.toFixed(1)} dB</span>
    </div>
  );
}

function EffectsBlock({ channel, inserts }: { channel: number; inserts: InsertState[] }) {
  const send = useSessionStore((s) => s.send);
  const removeInsertLocal = useProjectStore((s) => s.removeInsert);
  const setInsertParamLocal = useProjectStore((s) => s.setInsertParam);
  const [adding, setAdding] = useState(false);

  return (
    <div className="border-t border-daw-border pt-2 flex flex-col gap-1.5">
      <div className="flex items-center justify-between">
        <span className="text-[10px] uppercase tracking-widest text-[#888]">
          效果器（{inserts.length}）
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

      {inserts.map((insert) => (
        <InsertCard
          key={insert.id}
          insert={insert}
          onRemove={() => {
            removeInsertLocal(channel, insert.id);
            send({ type: 'insert.remove', channel, insertId: insert.id });
          }}
          onBypass={(bypassed) => {
            send({ type: 'insert.set_bypass', channel, insertId: insert.id, bypassed });
          }}
          onParam={(paramId, value) => {
            setInsertParamLocal(channel, insert.id, paramId, value);
            send({ type: 'insert.set_param', channel, insertId: insert.id, paramId, value });
          }}
        />
      ))}
    </div>
  );
}

function OverrideControls({ override }: { override: ChannelOverrideState }) {
  const send = useSessionStore((s) => s.send);
  const openPicker = useUiStore((s) => s.openInstrumentPicker);
  const updateChannel = useProjectStore((s) => s.updateChannel);
  const isVst3 = override.instrumentPath.toLowerCase().endsWith('.vst3');
  const guiSupported = isGuiSupported();

  return (
    <div className="flex items-center gap-1.5">
      <button
        type="button"
        onClick={() => {
          const next = !override.muted;
          updateChannel(override.channel, { muted: next });
          send({ type: 'channel.set_mute', channel: override.channel, muted: next });
        }}
        className={`w-7 h-7 text-[11px] font-bold rounded border transition-colors ${
          override.muted
            ? 'bg-red-500/80 border-red-400/60 text-white'
            : 'bg-daw-bg border-daw-border text-[#777] hover:text-white hover:border-[#555]'
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
        className={`w-7 h-7 text-[11px] font-bold rounded border transition-colors ${
          override.solo
            ? 'bg-yellow-500/80 border-yellow-400/60 text-black'
            : 'bg-daw-bg border-daw-border text-[#777] hover:text-white hover:border-[#555]'
        }`}
        title="独奏"
      >S</button>
      <button
        type="button"
        onClick={() => openPicker({ kind: 'override', channel: override.channel })}
        className="text-[11px] px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#ccc] transition-colors"
        title="更换这个通道的音色"
      >
        {override.instrumentName}
      </button>
      {isVst3 && guiSupported && (
        <button
          type="button"
          onClick={async () => {
            const result = await openPluginGui({ kind: 'override', channel: override.channel });
            if (!result.ok) console.error('[plugin-gui]', result.error);
          }}
          className="text-[11px] px-2 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors"
          title="打开插件原生界面"
        >插件界面</button>
      )}
      <button
        type="button"
        onClick={() => send({ type: 'channel.remove_override', channel: override.channel })}
        className="text-[11px] px-2 py-1 rounded text-[#888] hover:text-red-400 transition-colors"
        title="恢复默认音色"
      >× 恢复默认</button>
    </div>
  );
}

function InsertCard({
  insert, onRemove, onBypass, onParam,
}: {
  insert: InsertState;
  onRemove(): void;
  onBypass(bypassed: boolean): void;
  onParam(paramId: number, value: number): void;
}) {
  const [open, setOpen] = useState(true);
  const bypassed = insert.bypassed;
  return (
    <div className={`rounded border bg-daw-control/40 transition-colors ${
      bypassed ? 'border-daw-border/50 opacity-60' : 'border-daw-border'
    }`}>
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="w-3 text-[10px] text-[#888] hover:text-[#ccc]"
        >{open ? '▾' : '▸'}</button>
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${
          bypassed ? 'bg-[#555]' : 'bg-green-400'
        }`} />
        <span className={`text-xs flex-1 truncate ${
          bypassed ? 'text-[#888] line-through decoration-[#666]' : 'text-[#e0e0e0]'
        }`}>{insert.name}</span>
        <button
          type="button"
          onClick={() => onBypass(!bypassed)}
          className={`text-[9px] px-1.5 py-0.5 rounded border font-mono tracking-wider transition-colors ${
            bypassed
              ? 'bg-[#3a3a1a] border-yellow-700/60 text-yellow-300'
              : 'bg-daw-bg border-daw-border text-[#777] hover:text-[#ccc]'
          }`}
          title="切换旁通（音频绕过此效果）"
        >BYP</button>
        <button
          type="button"
          onClick={onRemove}
          className="text-[#666] hover:text-red-400 transition-colors px-1"
          title="移除"
        >×</button>
      </div>
      {open && insert.params.length > 0 && (
        <div className={`px-2.5 pb-2.5 pt-0.5 flex flex-col gap-1.5 border-t border-daw-border ${
          bypassed ? 'pointer-events-none' : ''
        }`}>
          {insert.params.map((p) => (
            <ParamSlider key={p.id} param={p} onChange={(v) => onParam(p.id, v)} />
          ))}
        </div>
      )}
    </div>
  );
}

