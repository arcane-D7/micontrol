import { Download } from 'lucide-react';

export function Navbar() {
  return (
    <nav className="lp-navbar">
      <a href="#top" className="lp-navbar-logo">
        <span className="lp-navbar-logo-icon">M</span>
        miControl
      </a>
      <ul className="lp-navbar-links">
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
      </ul>
      <a href="#download" className="lp-navbar-cta">
        <Download size={16} />
        Get Started
      </a>
    </nav>
  );
}
