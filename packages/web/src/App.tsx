import { useWebSocket } from './hooks/useWebSocket';
import { useTransportShortcuts } from './hooks/useTransportShortcuts';
import { ArrangeView } from './components/ArrangeView';
import { Mixer } from './components/Mixer';
import { TransportBar } from './components/TransportBar';
import { TrackInspector } from './components/TrackInspector';
import { VirtualKeyboard } from './components/VirtualKeyboard';

export function App() {
  useWebSocket();
  useTransportShortcuts();

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
    </div>
  );
}
