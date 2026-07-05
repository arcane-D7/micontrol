import { useEffect, useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import Lenis from 'lenis';
import { HeroSection } from './components/HeroSection';
import { NotebookTeardown } from './components/NotebookTeardown';
import { GearSection } from './components/GearSection';
import { SoftwareSection } from './components/SoftwareSection';
import { FeaturesSection } from './components/FeaturesSection';
import { DownloadSection } from './components/DownloadSection';
import { Footer } from './components/Footer';
import { Navbar } from './components/Navbar';
import { ProgressIndicator } from './components/ProgressIndicator';
import './styles/landing.css';

gsap.registerPlugin(ScrollTrigger, useGSAP);

export default function LandingApp() {
  const containerRef = useRef<HTMLDivElement>(null);
  const lenisRef = useRef<Lenis | null>(null);

  // ── Smooth scroll with Lenis ──────────────────────────────────────────────
  useEffect(() => {
    const lenis = new Lenis({
      duration: 1.2,
      easing: (t: number) => Math.min(1, 1.001 - Math.pow(2, -10 * t)),
      smoothWheel: true,
      touchMultiplier: 2,
    });
    lenisRef.current = lenis;
    // Expose Lenis globally so other components can listen to scroll events
    (window as unknown as Record<string, unknown>).__lenis = lenis;

    lenis.on('scroll', ScrollTrigger.update);
    // Also emit a custom event so components can react to Lenis scroll
    lenis.on('scroll', () => {
      window.dispatchEvent(new CustomEvent('lenis-scroll'));
    });

    const raf = (time: number) => {
      lenis.raf(time * 1000);
    };
    gsap.ticker.add(raf);
    gsap.ticker.lagSmoothing(0);

    return () => {
      gsap.ticker.remove(raf);
      lenis.destroy();
      lenisRef.current = null;
    };
  }, []);

  // ── Global scroll-driven timeline ─────────────────────────────────────────
  useGSAP(
    () => {
      // Refresh ScrollTrigger after images/fonts load
      ScrollTrigger.refresh();
    },
    { scope: containerRef },
  );

  return (
    <div ref={containerRef} className="landing-root">
      <ProgressIndicator />
      <Navbar />
      <HeroSection />
      <NotebookTeardown />
      <GearSection />
      <SoftwareSection />
      <FeaturesSection />
      <DownloadSection />
      <Footer />
    </div>
  );
}
