import { useState } from 'react';
import { Download, Menu, X } from 'lucide-react';

export function Navbar() {
  const [isOpen, setIsOpen] = useState(false);
  const closeMenu = () => setIsOpen(false);

  return (
    <nav className="lp-navbar" aria-label="Primary navigation">
      <a href="#top" className="lp-navbar-logo" onClick={closeMenu}>
        <span className="lp-navbar-logo-icon" aria-hidden="true">
          M
        </span>
        miControl
      </a>
      <button
        type="button"
        className="lp-navbar-menu-toggle"
        aria-expanded={isOpen}
        aria-controls="lp-navbar-links"
        aria-label={isOpen ? 'Close navigation' : 'Open navigation'}
        onClick={() => setIsOpen((open) => !open)}
      >
        {isOpen ? <X size={20} aria-hidden="true" /> : <Menu size={20} aria-hidden="true" />}
      </button>
      <ul id="lp-navbar-links" className={`lp-navbar-links ${isOpen ? 'is-open' : ''}`}>
        <li>
          <a href="#features" onClick={closeMenu}>
            Features
          </a>
        </li>
        <li>
          <a href="#software" onClick={closeMenu}>
            Software
          </a>
        </li>
        <li>
          <a href="#download" onClick={closeMenu}>
            Download
          </a>
        </li>
        <li>
          <a href="https://github.com/arcane-D7/micontrol" target="_blank" rel="noreferrer">
            GitHub
          </a>
        </li>
      </ul>
      <a href="#download" className="lp-navbar-cta" onClick={closeMenu}>
        <Download size={16} />
        Get Started
      </a>
    </nav>
  );
}
