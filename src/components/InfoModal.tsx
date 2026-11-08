import type { ReactNode } from 'react';
import Dialog from './ui/dialog';

interface Props {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
}

/**
 * InfoModal — a API de ajuda que já existe, agora sobre o primitivo `Dialog`.
 * O focus trap, o Escape e o restore de foco vivem no Dialog (uma vez, não
 * quatro). Visual: véu sem blur, painel hairline, zero sombra.
 */
export default function InfoModal({ open, onClose, title, children }: Props) {
  return (
    <Dialog open={open} onClose={onClose} title={title}>
      {children}
    </Dialog>
  );
}

/** A single glossary row inside InfoModal. */
export function InfoRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="mc-info-row">
      <div className="mc-info-row__label">{label}</div>
      <div className="mc-info-row__body">{children}</div>
    </div>
  );
}

/** A section divider with optional title used inside InfoModal. */
export function InfoSection({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <div className="mc-info-section">
      {title && <div className="mc-info-section__title">{title}</div>}
      {children}
    </div>
  );
}
