import { useEffect } from 'react';
import type { CSSProperties, HTMLAttributes, ReactNode } from 'react';

/** Absolutely-positioned box at exact logical coordinates (spec §0.1). */
export function Box(props: {
  x: number; y: number; w?: number; h?: number;
  className?: string; style?: CSSProperties; children?: ReactNode;
} & HTMLAttributes<HTMLDivElement>) {
  const { x, y, w, h, className, style, children, ...attributes } = props;
  return (
    <div
      {...attributes}
      className={'abs ' + (className ?? '')}
      style={{ left: x, top: y, width: w, height: h, ...style }}
    >
      {children}
    </div>
  );
}

/** Stone panel with gold trim (spec §0.4). */
export function Panel(props: {
  x: number; y: number; w: number; h: number;
  className?: string; style?: CSSProperties; children?: ReactNode;
}) {
  const { x, y, w, h, className, style, children } = props;
  return (
    <div
      className={'abs panel ' + (className ?? '')}
      style={{ left: x, top: y, width: w, height: h, ...style }}
    >
      {children}
    </div>
  );
}

/** Text block at exact coordinates (spec §0.3). */
export function Txt(props: {
  x: number; y: number; w?: number;
  size?: number; bold?: boolean; color?: string; align?: 'left' | 'center' | 'right';
  className?: string; style?: CSSProperties; children: ReactNode;
} & HTMLAttributes<HTMLDivElement>) {
  const { x, y, w, size = 14, bold, color, align = 'left', className, style, children, ...attributes } = props;
  return (
    <div
      {...attributes}
      className={'abs ' + (className ?? '')}
      style={{
        left: x, top: y, width: w,
        fontSize: size,
        fontWeight: bold ? 'bold' : 'normal',
        color: color ?? 'var(--text)',
        textAlign: align,
        ...style
      }}
    >
      {children}
    </div>
  );
}

/** Button at exact coordinates (spec §0.5). */
export function Btn(props: {
  x: number; y: number; w: number; h: number;
  label: string; onClick: () => void;
  disabled?: boolean; title?: string; size?: number;
}) {
  const { x, y, w, h, label, onClick, disabled, title, size = 14 } = props;
  return (
    <button
      className="btn"
      style={{ left: x, top: y, width: w, height: h, fontSize: size }}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {label}
    </button>
  );
}

/** Modal overlay (spec §0.8). */
export function Modal(props: {
  w: number; h: number;
  destructive?: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const { w, h, destructive, onClose, children } = props;
  const scale = Math.min(1, 780 / w, 580 / h);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !destructive) onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose, destructive]);
  return (
    <div
      className="backdrop"
      onClick={(e) => { if (!destructive && e.target === e.currentTarget) onClose(); }}
    >
      <div style={{ width: w * scale, height: h * scale }}>
        <div className="modal panel" role="dialog" aria-modal="true"
          style={{ width: w, height: h, transform: `scale(${scale})`, transformOrigin: 'top left' }}>
          {children}
          {!destructive && <button className="modal-close" aria-label="Close" onClick={onClose}>×</button>}
        </div>
      </div>
    </div>
  );
}

/** Loading state (spec §0.10). */
export function Loading() {
  return (
    <Box x={0} y={260} w={800}>
      <div className="spinner" />
      <div style={{ marginTop: 12, color: 'var(--text-dim)', fontSize: 12, textAlign: 'center' }}>
        Loading…
      </div>
    </Box>
  );
}

/** Relative time (spec §0.11). */
export function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return mins === 1 ? '1 minute ago' : mins + ' minutes ago';
  const hours = Math.floor(mins / 60);
  if (hours < 24) return hours === 1 ? '1 hour ago' : hours + ' hours ago';
  const days = Math.floor(hours / 24);
  if (days < 30) return days === 1 ? '1 day ago' : days + ' days ago';
  const months = Math.floor(days / 30);
  return months === 1 ? '1 month ago' : months + ' months ago';
}