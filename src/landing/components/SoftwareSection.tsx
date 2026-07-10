import { useRef, useState, useCallback, useEffect } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import { LiveAppPreview, PREVIEW_TABS } from './LiveAppPreview';

gsap.registerPlugin(ScrollTrigger, useGSAP);

export function SoftwareSection() {
  const sectionRef = useRef<HTMLElement>(null);
  const stickyRef = useRef<HTMLDivElement>(null);
  const mockupRef = useRef<HTMLDivElement>(null);
  const [activeTabId, setActiveTabId] = useState<string>(PREVIEW_TABS[0].id);

  const activeTab = PREVIEW_TABS.find((t) => t.id === activeTabId) ?? PREVIEW_TABS[0];

  const handleTabChange = useCallback((tabId: string) => {
    setActiveTabId(tabId);
  }, []);

  // ── Scale app-layout to fit mockup width (desktop shape preserved) ──────
  // The .app-layout is fixed at 720px wide (desktop width). On smaller
  // viewports the mockup shrinks, so we scale the app-layout down to fit.
  // This keeps the desktop layout intact (sidebar, grids, etc.) instead of
  // letting the responsive media queries in globals.css reflow the content.
  useEffect(() => {
    const mockup = mockupRef.current;
    if (!mockup) return;
    const appLayout = mockup.querySelector('.app-layout') as HTMLElement;
    if (!appLayout) return;

    const DESKTOP_WIDTH = 720;

    const updateScale = () => {
      const mockupWidth = mockup.clientWidth;
      const mockupHeight = mockup.clientHeight;
      // Only scale down when the mockup is narrower than the desktop layout.
      // Use a small threshold to avoid sub-pixel scaling at exact fit.
      if (mockupWidth < DESKTOP_WIDTH - 2) {
        const scale = mockupWidth / DESKTOP_WIDTH;
        appLayout.style.transform = `scale(${scale})`;
        // Compensate height: the app-layout is scaled visually, so we need
        // to set its CSS height to mockupHeight / scale so that after scaling
        // it visually fills the full mockup height.
        appLayout.style.height = `${mockupHeight / scale}px`;
      } else {
        appLayout.style.transform = 'none';
        appLayout.style.height = '100%';
      }
    };

    updateScale();
    const ro = new ResizeObserver(updateScale);
    ro.observe(mockup);
    window.addEventListener('resize', updateScale);

    return () => {
      ro.disconnect();
      window.removeEventListener('resize', updateScale);
    };
  }, []);

  // ── Scroll internal content to top when tab changes ─────────────────────
  // When the active tab changes (via click inside the mockup), reset the
  // content-area scroll position so the user sees the top of the new tab.
  useEffect(() => {
    const mockup = mockupRef.current;
    if (!mockup) return;
    const contentArea = mockup.querySelector('.content-area') as HTMLElement;
    if (contentArea) {
      contentArea.scrollTo({ top: 0, behavior: 'smooth' });
    }
  }, [activeTabId]);

  // ── Scroll sidebar to keep active tab visible ───────────────────────────
  // When the active tab changes, scroll the sidebar so the active tab item
  // is always visible within the sidebar's scroll viewport.
  useEffect(() => {
    const mockup = mockupRef.current;
    if (!mockup) return;

    const scrollSidebar = () => {
      const sidebar = mockup.querySelector('.sidebar') as HTMLElement;
      if (!sidebar) return;
      const activeItem = sidebar.querySelector('.sidebar-item.active') as HTMLElement;
      if (!activeItem) return;

      const sidebarRect = sidebar.getBoundingClientRect();
      const itemRect = activeItem.getBoundingClientRect();
      const itemTop = itemRect.top - sidebarRect.top;
      const itemBottom = itemRect.bottom - sidebarRect.top;
      const viewHeight = sidebar.clientHeight;

      if (itemTop < 0) {
        sidebar.scrollTop += itemTop - 8;
      } else if (itemBottom > viewHeight) {
        sidebar.scrollTop += itemBottom - viewHeight + 8;
      }
    };

    // Try multiple times to handle async React rendering of the Sidebar
    // component inside LiveAppPreview → MainWindow.
    const raf1 = requestAnimationFrame(() => {
      scrollSidebar();
      requestAnimationFrame(scrollSidebar);
    });
    const timeout = setTimeout(scrollSidebar, 150);

    return () => {
      cancelAnimationFrame(raf1);
      clearTimeout(timeout);
    };
  }, [activeTabId]);

  // ── Hover-based focus for internal scroll ────────────────────────────────
  // When the cursor enters the miControl window, we programmatically focus the
  // element under the cursor (content-area or sidebar) so that wheel events
  // scroll the hovered region. When the cursor leaves, we blur so the landing
  // page (Lenis) regains scroll control.
  useEffect(() => {
    const mockup = mockupRef.current;
    if (!mockup) return;

    const contentArea = mockup.querySelector('.content-area') as HTMLElement;
    const sidebar = mockup.querySelector('.sidebar') as HTMLElement;

    const handleMouseEnter = () => {
      if (contentArea) {
        contentArea.focus({ preventScroll: true });
        contentArea.setAttribute('tabindex', '-1');
      }
    };

    const handleMouseLeave = () => {
      if (contentArea) contentArea.blur();
      if (sidebar) sidebar.blur();
    };

    // Track which sub-region the cursor is over
    const handleContentEnter = () => {
      if (contentArea) {
        contentArea.setAttribute('tabindex', '-1');
        contentArea.focus({ preventScroll: true });
      }
    };

    const handleSidebarEnter = () => {
      if (sidebar) {
        sidebar.setAttribute('tabindex', '-1');
        sidebar.focus({ preventScroll: true });
      }
    };

    mockup.addEventListener('mouseenter', handleMouseEnter, { passive: true });
    mockup.addEventListener('mouseleave', handleMouseLeave, { passive: true });
    if (contentArea) {
      contentArea.addEventListener('mouseenter', handleContentEnter, { passive: true });
    }
    if (sidebar) {
      sidebar.addEventListener('mouseenter', handleSidebarEnter, { passive: true });
    }

    return () => {
      mockup.removeEventListener('mouseenter', handleMouseEnter);
      mockup.removeEventListener('mouseleave', handleMouseLeave);
      if (contentArea) {
        contentArea.removeEventListener('mouseenter', handleContentEnter);
      }
      if (sidebar) {
        sidebar.removeEventListener('mouseenter', handleSidebarEnter);
      }
    };
  }, []);

  // ── Internal scroll inside the app preview window ───────────────────────
  // When the cursor is over the miControl window, wheel events should scroll
  // the hovered region (content-area or sidebar) instead of the landing page.
  // We intercept the wheel event in capture phase (before Lenis), prevent
  // default so Lenis doesn't scroll the page, and manually scroll the target.
  useEffect(() => {
    const mockup = mockupRef.current;
    if (!mockup) return;

    const handleWheel = (e: WheelEvent) => {
      // Determine which scrollable element is under the cursor
      const target = e.target as HTMLElement;
      const isInSidebar = target.closest('.sidebar');
      const isInContent = target.closest('.content-area');

      let scrollTarget: HTMLElement | null = null;

      if (isInSidebar) {
        scrollTarget = mockup.querySelector('.sidebar') as HTMLElement;
      } else if (isInContent) {
        scrollTarget = mockup.querySelector('.content-area') as HTMLElement;
      }

      if (!scrollTarget) return;

      const canScrollUp = scrollTarget.scrollTop > 0;
      const canScrollDown =
        scrollTarget.scrollTop + scrollTarget.clientHeight < scrollTarget.scrollHeight - 1;

      // If the target can handle the scroll direction, intercept
      if ((e.deltaY < 0 && canScrollUp) || (e.deltaY > 0 && canScrollDown)) {
        e.preventDefault();
        e.stopPropagation();
        scrollTarget.scrollTop += e.deltaY;
      }
    };

    // Capture phase + passive:false so we can call preventDefault
    mockup.addEventListener('wheel', handleWheel, { passive: false, capture: true });
    return () => mockup.removeEventListener('wheel', handleWheel, { capture: true });
  }, []);

  // ── Entrance animations ───────────────────────────────────────────────────
  useGSAP(
    () => {
      gsap.from('.software-mockup', {
        y: 80,
        opacity: 0,
        duration: 1,
        scrollTrigger: {
          trigger: sectionRef.current,
          start: 'top 80%',
          end: 'top 20%',
          scrub: 1,
        },
      });

      gsap.from('.software-info > *', {
        y: 40,
        opacity: 0,
        duration: 0.8,
        stagger: 0.1,
        scrollTrigger: {
          trigger: sectionRef.current,
          start: 'top 60%',
          end: 'top 20%',
          scrub: 1,
        },
      });
    },
    { scope: sectionRef },
  );

  return (
    <section ref={sectionRef} className="software-section" id="software">
      <div ref={stickyRef} className="software-sticky">
        <div ref={mockupRef} className="software-mockup">
          <LiveAppPreview activeTab={activeTabId} onTabChange={handleTabChange} />
        </div>

        <div className="software-info">
          <span className="lp-section-tag">The Software</span>
          <h2>{activeTab.title}</h2>
          <p>{activeTab.description}</p>
        </div>
      </div>
    </section>
  );
}
