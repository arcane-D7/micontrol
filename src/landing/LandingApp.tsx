import {
  lazy,
  Suspense,
  useEffect,
  useRef,
  useState,
  type ComponentType,
  type LazyExoticComponent,
} from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import Lenis from 'lenis';
import { HeroSection } from './components/HeroSection';
import { SoftwareSection } from './components/SoftwareSection';
import { FeaturesSection } from './components/FeaturesSection';
import { DownloadSection } from './components/DownloadSection';
import { Footer } from './components/Footer';
import { Navbar } from './components/Navbar';
import { ProgressIndicator } from './components/ProgressIndicator';
import './styles/landing.css';

gsap.registerPlugin(ScrollTrigger, useGSAP);

const NotebookTeardown = lazy(() =>
  import('./components/NotebookTeardown').then(({ NotebookTeardown }) => ({
    default: NotebookTeardown,
  })),
);
const GearSection = lazy(() =>
  import('./components/GearSection').then(({ GearSection }) => ({ default: GearSection })),
);

function DeferredScene({
  Scene,
  className,
  label,
  minHeight,
}: {
  Scene: LazyExoticComponent<ComponentType>;
  className: string;
  label: string;
  minHeight: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isNearViewport, setIsNearViewport] = useState(false);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    if (typeof IntersectionObserver === 'undefined') {
      setIsNearViewport(true);
      return;
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setIsNearViewport(true);
          observer.disconnect();
        }
      },
      { rootMargin: '120px 0px' },
    );
    observer.observe(container);

    return () => observer.disconnect();
  }, []);

  return (
    <div
      ref={containerRef}
      className={`deferred-scene ${className}`}
      style={{ minHeight: isNearViewport ? undefined : minHeight }}
    >
      {isNearViewport ? (
        <Suspense
          fallback={
            <div className="scene-placeholder" role="status" aria-live="polite">
              <span className="model-loader-spinner" aria-hidden="true" />
              <span>{label}</span>
            </div>
          }
        >
          <Scene />
        </Suspense>
      ) : (
        <div className="scene-placeholder" role="status" aria-live="polite">
          <span className="model-loader-spinner" aria-hidden="true" />
          <span>{label}</span>
        </div>
      )}
    </div>
  );
}

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
      <main id="main-content">
        <HeroSection />
        <DeferredScene
          Scene={NotebookTeardown}
          className="deferred-scene-teardown"
          label="Notebook view loads as you scroll"
          minHeight="400vh"
        />
        <DeferredScene
          Scene={GearSection}
          className="deferred-scene-gear"
          label="Hardware view loads as you scroll"
          minHeight="500px"
        />
        <SoftwareSection />
        <FeaturesSection />
        <DownloadSection />
      </main>
      <Footer />
    </div>
  );
}
