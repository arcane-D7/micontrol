import { Heart } from 'lucide-react';

export function Footer() {
  return (
    <footer className="lp-footer">
      <div className="lp-footer-container">
        <div className="lp-footer-brand">
          <span
            className="lp-navbar-logo-icon"
            style={{ width: '28px', height: '28px', fontSize: '0.85rem' }}
          >
            M
          </span>
          miControl
        </div>
        <ul className="lp-footer-links">
          <li>
            <a href="#features">Features</a>
          </li>
          <li>
            <a href="#software">Software</a>
          </li>
          <li>
            <a href="#download">Download</a>
          </li>
          <li>
            <a href="https://github.com/arcane-D7/micontrol" target="_blank" rel="noreferrer">
              GitHub
            </a>
          </li>
          <li>
            <a href="https://github.com/arcane-D7/micontrol/blob/main/LICENSE">License</a>
          </li>
        </ul>
        <div className="lp-footer-copy">
          © 2026 miControl · MIT License · Made with{' '}
          <Heart size={12} style={{ display: 'inline', color: 'var(--lp-orange)' }} /> in Portugal
        </div>
      </div>
    </footer>
  );
}
