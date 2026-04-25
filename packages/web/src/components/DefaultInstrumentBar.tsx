import { useState } from 'react';
import { useUiStore } from '../stores/ui';
import { isGuiSupported, openPluginGui } from '../services/pluginGui';

interface DefaultInstrumentBarProps {
  instrumentPath: string | null;
}

export function DefaultInstrumentBar({ instrumentPath }: DefaultInstrumentBarProps) {
  const open = useUiStore((s) => s.openInstrumentPicker);
  const name = instrumentPath ? (instrumentPath.split('/').pop() ?? instrumentPath) : null;
  const isVst3 = instrumentPath?.toLowerCase().endsWith('.vst3') ?? false;
  const guiSupported = isGuiSupported();
  const [guiError, setGuiError] = useState<string | null>(null);

  return (
    <section className="bg-daw-panel border border-daw-border rounded-lg p-4">
      <div className="text-[11px] uppercase tracking-widest text-[#888] font-semibold mb-2">
        默认音色
      </div>
      <div className="flex items-center gap-3 flex-wrap">
        <button
          type="button"
          onClick={() => open({ kind: 'default' })}
          className={`px-4 py-2 rounded text-sm font-medium transition-colors ${
            name
              ? 'bg-daw-control hover:bg-daw-border text-[#e0e0e0]'
              : 'bg-daw-accent hover:bg-daw-accent/80 text-white'
          }`}
        >
          {name ? <>🎹 {name}</> : <>🎹 选择默认音色…</>}
        </button>
        <div className="flex-1 min-w-[200px] text-[11px] text-[#888]">
          {name
            ? '所有未单独设置音色的通道都用它播放（GM SoundFont 内部会按 MIDI 的 Program Change 自动切换音色）'
            : 'SF2 / VST3 / CLAP — 推荐选 GeneralUser_GS 这类 GM SoundFont'}
        </div>
        {isVst3 && guiSupported && (
          <button
            type="button"
            onClick={async () => {
              setGuiError(null);
              const err = await openPluginGui({ kind: 'default' });
              if (err) setGuiError(err);
            }}
            className="text-[11px] px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors"
            title="打开插件原生界面"
          >
            🎛 GUI
          </button>
        )}
        {name && (
          <button
            type="button"
            onClick={() => open({ kind: 'default' })}
            className="text-[11px] px-2.5 py-1 rounded bg-daw-control hover:bg-daw-border text-[#aaa] transition-colors"
          >
            更换…
          </button>
        )}
      </div>
      {guiError !== null && (
        <div className="mt-2 text-[11px] text-red-400">{guiError}</div>
      )}
    </section>
  );
}
