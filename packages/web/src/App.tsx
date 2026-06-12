import { useCallback, useEffect } from 'react';
import { useWebSocket } from './hooks/useWebSocket';
import { useTransportShortcuts } from './hooks/useTransportShortcuts';
import { useProjectShortcuts } from './hooks/useProjectShortcuts';
import { useUiStore } from './stores/ui';
import { useSessionStore } from './stores/session';
import { useProjectFileStore } from './stores/projectFile';
import { PlayerView } from './components/PlayerView';
import { InstrumentSelector } from './components/InstrumentSelector';

export function App() {
  useWebSocket();
  useTransportShortcuts();
  useProjectShortcuts();

  const target = useUiStore((s) => s.instrumentTarget);
  const close = useUiStore((s) => s.closeInstrumentPicker);
  const send = useSessionStore((s) => s.send);
  const projectPath = useProjectFileStore((s) => s.path);
  const dirty = useProjectFileStore((s) => s.dirty);

  // Startup state restoration lives in the transport boot sequence
  // now: the autosave journal carries the full last session (including
  // which project was open), superseding the old "reopen last file"
  // behaviour — and unlike it, survives crashes with unsaved work.

  // Reflect the open project + dirty marker in the window title — the
  // standard "filename.mlsession — moonlitt" pattern with an asterisk
  // when there are unsaved changes.
  useEffect(() => {
    const name = projectPath
      ? projectPath.split('/').pop()?.replace(/\.mlsession$/i, '') ?? 'untitled'
      : 'untitled';
    document.title = `${dirty ? '• ' : ''}${name} — moonlitt`;
  }, [projectPath, dirty]);

  const handleLoad = useCallback(
    (path: string) => {
      if (target === null) return;
      if (target.kind === 'default') {
        send({ type: 'default.set_instrument', path });
      } else {
        send({ type: 'channel.set_override', channel: target.channel, path });
      }
      useProjectFileStore.getState().markDirty();
      close();
    },
    [target, send, close],
  );

  return (
    <>
      <PlayerView />
      <InstrumentSelector
        open={target !== null}
        targetKind={target?.kind ?? null}
        onLoad={handleLoad}
        onClose={close}
      />
    </>
  );
}
