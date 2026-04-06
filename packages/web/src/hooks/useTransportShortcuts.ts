import { useEffect } from 'react';
import { useSessionStore } from '../stores/session';
import { useTransportStore } from '../stores/transport';

export function useTransportShortcuts(): void {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (isInputFocused(e)) return;

      if (e.code === 'Space') {
        e.preventDefault();
        const playing = useTransportStore.getState().playing;
        const send = useSessionStore.getState().send;
        if (playing) {
          send({ type: 'transport.stop' });
        } else {
          send({ type: 'transport.play' });
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);
}

function isInputFocused(e: KeyboardEvent): boolean {
  const target = e.target as HTMLElement;
  return (
    target.tagName === 'INPUT' ||
    target.tagName === 'TEXTAREA' ||
    target.tagName === 'SELECT' ||
    target.isContentEditable
  );
}
