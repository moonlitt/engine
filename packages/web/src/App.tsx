import { useWebSocket } from './hooks/useWebSocket';
import { useSessionStore } from './stores/session';

export function App() {
  useWebSocket();
  const connected = useSessionStore((s) => s.connected);

  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e0e0e0] font-sans text-sm">
      {/* Transport Bar */}
      <div className="h-12 bg-daw-panel border-b border-daw-border flex items-center px-4">
        <span className="text-daw-accent font-bold">moonlitt</span>
          <div className="ml-3 flex items-center gap-1.5">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'}`} />
            <span className="text-xs text-[#888]">{connected ? 'connected' : 'offline'}</span>
          </div>
      </div>

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
      <div className="h-40 bg-daw-panel border-t-2 border-daw-border p-3">
        <div className="text-[#555] text-xs">Mixer</div>
      </div>

      {/* Virtual Keyboard */}
      <div className="h-16 bg-daw-surface border-t border-daw-border p-2">
        <div className="text-[#555] text-xs">Virtual Keyboard</div>
      </div>
    </div>
  );
}
