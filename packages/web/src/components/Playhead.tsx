interface PlayheadProps {
  positionPx: number;
  scrollLeft: number;
}

export function Playhead({ positionPx, scrollLeft }: PlayheadProps) {
  const visible = positionPx - scrollLeft;

  // Hide when off-screen to the left
  if (visible < 0) {
    return null;
  }

  return (
    <div
      className="absolute top-0 bottom-0 w-px bg-daw-accent pointer-events-none z-10"
      style={{ left: `${120 + visible}px` }}
    >
      {/* Triangle head */}
      <div
        className="absolute -top-0.5 -left-[4px] w-0 h-0
          border-l-[4px] border-l-transparent
          border-r-[4px] border-r-transparent
          border-t-[5px] border-t-daw-accent"
      />
    </div>
  );
}
