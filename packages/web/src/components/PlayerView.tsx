import { useProjectStore } from '../stores/project';
import { useSessionStore } from '../stores/session';
import { Transport } from './Transport';
import { Ruler } from './Ruler';
import { MidiPanel } from './MidiPanel';
import { DefaultInstrumentBar } from './DefaultInstrumentBar';
import { ChannelRow } from './ChannelRow';
import { SendBusRack } from './SendBusRack';
import { PatchBrowser } from './PatchBrowser';

export function PlayerView() {
  const connected = useSessionStore((s) => s.connected);
  const midi = useProjectStore((s) => s.midi);
  const overrides = useProjectStore((s) => s.overrides);
  const defaultInstrumentPath = useProjectStore((s) => s.defaultInstrumentPath);
  const defaultPatchName = useProjectStore((s) => s.defaultPatchName);

  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e6e2dc] font-ui">
      <Transport />
      <Ruler />
      <PatchPendingBanner />

      <main className="flex-1 overflow-y-auto">
        <div className="max-w-[1040px] mx-auto px-5 py-4 flex flex-col gap-4">
          <MidiPanel midi={midi} />

          {midi !== null && (
            <>
              <DefaultInstrumentBar
                instrumentPath={defaultInstrumentPath}
                patchName={defaultPatchName}
              />

              <SendBusRack />

              <section className="pb-8">
                <div className="lcd-label text-[#7c776c] mb-2 px-1 !text-[10px]">
                  轨道 · {midi.channels.length}
                </div>
                <div className="flex flex-col gap-2">
                  {midi.channels.map((ch, i) => {
                    const ov = overrides.find((o) => o.channel === ch.channel) ?? null;
                    return (
                      <div
                        key={ch.channel}
                        className="strip-in"
                        style={{ animationDelay: `${i * 28}ms` }}
                      >
                        <ChannelRow
                          info={ch}
                          override={ov}
                          defaultInstrumentPath={defaultInstrumentPath}
                        />
                      </div>
                    );
                  })}
                </div>
              </section>
            </>
          )}

          {!connected && (
            <div className="text-center text-[#6b675f] text-xs mt-6">正在连接音频引擎…</div>
          )}
        </div>
      </main>

      <PatchBrowser />
    </div>
  );
}

/**
 * Sample-streamer guidance: the plug-in is loaded but silent until a
 * patch is picked. The backend already opened its GUI window — this
 * strip explains why there's no sound and disappears the moment the
 * first patch capture arrives (then it's remembered forever).
 */
function PatchPendingBanner() {
  const pending = useProjectStore((s) => s.patchPending);
  const clear = useProjectStore((s) => s.setPatchPending);
  if (!pending) return null;
  const name = pending.path.split('/').pop()?.replace(/\.vst3$/i, '') ?? '插件';
  const where = pending.target === 'default' ? '默认乐器' : `通道 ${(pending.target as number) + 1}`;
  return (
    <div className="shrink-0 flex items-center gap-2.5 px-4 py-2 bg-[#2e2a1d] border-b border-state-solo/30 text-xs text-[#e8dcb0]">
      <span className="pulse-dot w-2 h-2 rounded-full bg-state-solo shrink-0" />
      <span className="flex-1">
        <b>{name}</b>（{where}）是采样流送插件，选好音色才会出声 —— 已为你打开它的界面，
        在里面挑一个音色即可（moonlitt 会记住，下次直接出声）。
      </span>
      <button
        type="button"
        onClick={() => clear(null)}
        className="text-[#a89a6c] hover:text-[#e8dcb0] px-1.5"
        title="知道了"
      >
        ✕
      </button>
    </div>
  );
}
