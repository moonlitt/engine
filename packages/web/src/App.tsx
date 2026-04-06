export function App() {
  return (
    <div className="h-screen flex flex-col bg-daw-bg text-[#e0e0e0] font-sans text-sm">
      {/* Transport Bar */}
      <div className="h-12 bg-daw-panel border-b border-daw-border flex items-center px-4">
        <span className="text-daw-accent font-bold">moonlitt</span>
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
