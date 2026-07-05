import { useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import { Download, Code2 } from 'lucide-react';

gsap.registerPlugin(ScrollTrigger, useGSAP);

export function HeroSection() {
  const ref = useRef<HTMLElement>(null);

  useGSAP(
    () => {
      const tl = gsap.timeline({ defaults: { ease: 'power3.out' } });
      tl.from('.hero-badge', { y: 20, opacity: 0, duration: 0.6 })
        .from('.hero-title-line', { y: 40, opacity: 0, duration: 0.8, stagger: 0.15 }, '-=0.3')
        .from('.hero-subtitle', { y: 20, opacity: 0, duration: 0.6 }, '-=0.4')
        .from('.hero-cta-group > *', { y: 20, opacity: 0, duration: 0.5, stagger: 0.1 }, '-=0.3')
        .from('.hero-scroll-hint', { opacity: 0, duration: 0.5 }, '-=0.2');

      // Parallax glow
      gsap.to('.hero-bg-glow', {
        yPercent: 30,
        ease: 'none',
        scrollTrigger: {
          trigger: '.hero-section',
          start: 'top top',
          end: 'bottom top',
          scrub: 1,
        },
      });
    },
    { scope: ref },
  );

  return (
    <section ref={ref} className="hero-section" id="top">
      <div className="hero-bg-glow" />
      <div className="hero-content">
        <div className="hero-badge">
          <span className="hero-badge-dot" />
          Open Source · Tauri 2 · Windows
        </div>
        <h1 className="hero-title">
          <span className="hero-title-line">Take Control of</span>
          <br />
          <span className="hero-title-line hero-title-accent">Your Xiaomi Notebook</span>
        </h1>
        <p className="hero-subtitle">
          miControl unlocks the full potential of your Xiaomi Notebook Pro — performance modes, fan
          control, battery insights, and hardware diagnostics in one beautiful desktop app.
        </p>
        <div className="hero-cta-group">
          <a href="#download" className="hero-cta-primary">
            <Download size={18} />
            Download Free
          </a>
          <a
            href="https://github.com/arcane-D7/micontrol"
            target="_blank"
            rel="noreferrer"
            className="hero-cta-secondary"
          >
            <Code2 size={18} />
            View Source
          </a>
        </div>
      </div>
      <div className="hero-scroll-hint">
        Scroll to explore
        <div className="hero-scroll-hint-line" />
      </div>
    </section>
  );
}
