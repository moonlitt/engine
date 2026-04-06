import { useWebSocket } from './hooks/useWebSocket';
import { Mixer } from './components/Mixer';
import { TransportBar } from './components/TransportBar';
import { VirtualKeyboard } from './components/VirtualKeyboard';

export function App() {
  useWebSocket();

  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e0e0e0] font-sans text-sm">
      {/* Transport Bar */}
      <TransportBar />

      {/* Main Area */}
      <div className="flex-1 flex overflow-hidden">
        <div className="flex-1 bg-daw-surface">
          {/* Arrange View placeholder */}
          <div className="flex items-center justify-center h-full text-[#555]">
            Arrange View
          </div>
        </div>
        <div className="w-[220px] bg-daw-panel border-l border-daw-border p-3">
          {/* Track Inspector placeholder */}
          <div className="text-[#555] text-xs">Track Inspector</div>
        </div>
      </div>

      {/* Mixer */}
      <Mixer />

      {/* Virtual Keyboard */}
      <VirtualKeyboard />
    </div>
  );
}
