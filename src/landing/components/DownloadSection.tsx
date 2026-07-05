import { useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import { Download, Code2 } from 'lucide-react';

gsap.registerPlugin(ScrollTrigger, useGSAP);

export function DownloadSection() {
  const sectionRef = useRef<HTMLElement>(null);

  useGSAP(
    () => {
      gsap.from('.download-container > *', {
        y: 40,
        opacity: 0,
        duration: 0.8,
        stagger: 0.12,
        scrollTrigger: {
          trigger: sectionRef.current,
          start: 'top 70%',
          end: 'top 30%',
          scrub: 1,
        },
      });

      // Glow pulse
      gsap.to('.download-glow', {
        scale: 1.2,
        opacity: 0.8,
        duration: 3,
        repeat: -1,
        yoyo: true,
        ease: 'sine.inOut',
      });
    },
    { scope: sectionRef },
  );

  return (
    <section ref={sectionRef} className="download-section" id="download">
      <div className="download-glow" />
      <div className="download-container">
        <span className="lp-section-tag">Get Started</span>
        <h2>Take Control Today</h2>
        <p>
          Download miControl for free. Open source, no ads, no tracking. Just pure hardware control.
        </p>
        <div className="download-buttons">
          <a
            href="https://github.com/arcane-D7/micontrol/releases/latest"
            target="_blank"
            rel="noreferrer"
            className="download-btn download-btn-primary"
          >
            <Download size={20} />
            Download for Windows
          </a>
          <a
            href="https://github.com/arcane-D7/micontrol"
            target="_blank"
            rel="noreferrer"
            className="download-btn download-btn-secondary"
          >
            <Code2 size={20} />
            View on GitHub
          </a>
        </div>
        <div className="download-meta">v0.1.4 · Windows 10/11 x64 · 5.3 MB · MIT License</div>
      </div>
    </section>
  );
}
