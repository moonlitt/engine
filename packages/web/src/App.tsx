import { useCallback } from 'react';
import { useWebSocket } from './hooks/useWebSocket';
import { useTransportShortcuts } from './hooks/useTransportShortcuts';
import { useUiStore } from './stores/ui';
import { useSessionStore } from './stores/session';
import { ArrangeView } from './components/ArrangeView';
import { Mixer } from './components/Mixer';
import { TransportBar } from './components/TransportBar';
import { TrackInspector } from './components/TrackInspector';
import { VirtualKeyboard } from './components/VirtualKeyboard';
import { InstrumentSelector } from './components/InstrumentSelector';

export function App() {
  useWebSocket();
  useTransportShortcuts();

  const targetTrackId = useUiStore((s) => s.instrumentSelectorTrackId);
  const closeSelector = useUiStore((s) => s.closeInstrumentSelector);
  const send = useSessionStore((s) => s.send);

  const handleLoad = useCallback(
    (path: string) => {
      if (targetTrackId === null) return;
      send({ type: 'track.load_instrument', trackId: targetTrackId, path });
      closeSelector();
    },
    [targetTrackId, send, closeSelector],
  );

  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e0e0e0] font-sans text-sm">
      {/* Transport Bar */}
      <TransportBar />

      {/* Main Area */}
      <div className="flex-1 flex overflow-hidden">
        <div className="flex-1 bg-daw-surface">
          <ArrangeView />
        </div>
        <div className="w-[220px] bg-daw-panel border-l border-daw-border p-3">
          <TrackInspector />
        </div>
      </div>

      {/* Mixer */}
      <Mixer />

      {/* Virtual Keyboard */}
      <VirtualKeyboard />

      {/* Global instrument-selector modal — opened by lane CTA or inspector */}
      <InstrumentSelector
        open={targetTrackId !== null}
        onLoad={handleLoad}
        onClose={closeSelector}
      />
    </div>
  );
}
