import { useCallback } from 'react';
import { useWebSocket } from './hooks/useWebSocket';
import { useTransportShortcuts } from './hooks/useTransportShortcuts';
import { useUiStore } from './stores/ui';
import { useSessionStore } from './stores/session';
import { PlayerView } from './components/PlayerView';
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
    <>
      <PlayerView />
      <InstrumentSelector
        open={targetTrackId !== null}
        onLoad={handleLoad}
        onClose={closeSelector}
      />
    </>
  );
}
