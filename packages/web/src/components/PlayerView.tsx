import { useTransportStore } from '../stores/transport';
import { useSessionStore } from '../stores/session';
import { useProjectStore } from '../stores/project';
import { TopBar } from './TopBar';
import { MidiPanel } from './MidiPanel';
import { DefaultInstrumentBar } from './DefaultInstrumentBar';
import { ChannelRow } from './ChannelRow';

export function PlayerView() {
  const connected = useSessionStore((s) => s.connected);
  const playing = useTransportStore((s) => s.playing);
  const position = useTransportStore((s) => s.position);
  const bpm = useTransportStore((s) => s.bpm);
  const send = useSessionStore((s) => s.send);

  const midi = useProjectStore((s) => s.midi);
  const overrides = useProjectStore((s) => s.overrides);
  const defaultInstrumentPath = useProjectStore((s) => s.defaultInstrumentPath);
  const defaultPatchName = useProjectStore((s) => s.defaultPatchName);

  return (
    <div className="h-screen overflow-y-auto bg-daw-bg text-[#e0e0e0] font-sans">
      <TopBar
        connected={connected}
        playing={playing}
        position={position}
        bpm={bpm}
        onPlay={() => send({ type: playing ? 'transport.stop' : 'transport.play' })}
        onStop={() => send({ type: 'transport.stop' })}
        onBpmChange={(v) => send({ type: 'transport.set_bpm', bpm: v })}
      />

      <div className="max-w-[840px] mx-auto px-6 py-5 flex flex-col gap-5">
        <MidiPanel midi={midi} />

        {midi !== null && (
          <>
            <DefaultInstrumentBar
              instrumentPath={defaultInstrumentPath}
              patchName={defaultPatchName}
            />

            <section>
              <div className="text-[11px] uppercase tracking-widest text-[#888] font-semibold mb-2 px-1">
                通道（来自 MIDI 文件）
              </div>
              <div className="flex flex-col gap-3">
                {midi.channels.map((ch) => {
                  const ov = overrides.find((o) => o.channel === ch.channel) ?? null;
                  return (
                    <ChannelRow
                      key={ch.channel}
                      info={ch}
                      override={ov}
                      defaultInstrumentPath={defaultInstrumentPath}
                    />
                  );
                })}
              </div>
            </section>
          </>
        )}

        {!connected && (
          <div className="text-center text-[#666] text-xs mt-4">正在连接音频引擎…</div>
        )}
      </div>
    </div>
  );
}
