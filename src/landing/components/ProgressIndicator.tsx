import { useEffect, useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';

gsap.registerPlugin(ScrollTrigger);

export function ProgressIndicator() {
  const barRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!barRef.current) return;

    const st = ScrollTrigger.create({
      start: 0,
      end: 'max',
      onUpdate: (self) => {
        gsap.to(barRef.current, {
          scaleX: self.progress,
          duration: 0.1,
          overwrite: true,
        });
      },
    });

    return () => st.kill();
  }, []);

  return (
    <div className="scroll-progress">
      <div ref={barRef} className="scroll-progress-bar" />
    </div>
  );
}
