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

  const target = useUiStore((s) => s.instrumentTarget);
  const close = useUiStore((s) => s.closeInstrumentPicker);
  const send = useSessionStore((s) => s.send);

  const handleLoad = useCallback(
    (path: string) => {
      if (target === null) return;
      if (target.kind === 'default') {
        send({ type: 'default.set_instrument', path });
      } else {
        send({ type: 'channel.set_override', channel: target.channel, path });
      }
      close();
    },
    [target, send, close],
  );

  return (
    <>
      <PlayerView />
      <InstrumentSelector
        open={target !== null}
        onLoad={handleLoad}
        onClose={close}
      />
    </>
  );
}
