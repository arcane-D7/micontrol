import { useRef, Suspense, useEffect } from 'react';
import { Canvas } from '@react-three/fiber';
import { useGLTF, Environment, ContactShadows, Float } from '@react-three/drei';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import * as THREE from 'three';

gsap.registerPlugin(ScrollTrigger, useGSAP);

// ── 3D Model components ─────────────────────────────────────────────────────

function LaptopModel({ progressRef }: { progressRef: React.RefObject<number> }) {
  const groupRef = useRef<THREE.Group>(null);
  const { scene } = useGLTF(`${import.meta.env.BASE_URL}landing/models/laptop.glb`) as unknown as {
    scene: THREE.Group;
  };

  useEffect(() => {
    if (!groupRef.current) return;

    // Clone and center the model
    const model = scene.clone(true);
    const box = new THREE.Box3().setFromObject(model);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const maxDim = Math.max(size.x, size.y, size.z);
    const scale = 3 / maxDim;
    model.position.sub(center.multiplyScalar(scale));
    model.scale.setScalar(scale);
    groupRef.current.add(model);

    const ctx = gsap.context(() => {
      // Scroll-driven animation: rotate
      gsap.to(groupRef.current!.rotation, {
        y: Math.PI * 2,
        scrollTrigger: {
          trigger: '.teardown-section',
          start: 'top top',
          end: 'bottom bottom',
          scrub: 1.5,
          onUpdate: (self) => {
            if (progressRef.current !== undefined) {
              (progressRef as React.MutableRefObject<number>).current = self.progress;
            }
          },
        },
      });

      // Tilt as user scrolls
      gsap.to(groupRef.current!.rotation, {
        x: -0.3,
        scrollTrigger: {
          trigger: '.teardown-section',
          start: 'top top',
          end: '50% bottom',
          scrub: 1,
        },
      });

      // Scale down + fade out as motherboard appears (0% → 30% scroll)
      const laptopProxy = { val: 1 };
      gsap.to(laptopProxy, {
        val: 0,
        scrollTrigger: {
          trigger: '.teardown-section',
          start: '15% top',
          end: '35% top',
          scrub: 1,
          onUpdate: () => {
            if (groupRef.current) {
              groupRef.current.scale.setScalar(laptopProxy.val);
              // Hide completely when fully scaled out
              groupRef.current.visible = laptopProxy.val > 0.01;
            }
          },
        },
      });
    });

    return () => ctx.revert();
  }, [scene, progressRef]);

  return <group ref={groupRef}>{/* The model is added dynamically in useEffect */}</group>;
}

function MotherboardModel() {
  const groupRef = useRef<THREE.Group>(null);
  const { scene } = useGLTF(
    `${import.meta.env.BASE_URL}landing/models/motherboard.glb`,
  ) as unknown as { scene: THREE.Group };

  useEffect(() => {
    if (!groupRef.current) return;

    const model = scene.clone(true);
    const box = new THREE.Box3().setFromObject(model);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const maxDim = Math.max(size.x, size.y, size.z);
    const scale = 3 / maxDim;
    model.position.sub(center.multiplyScalar(scale));
    model.scale.setScalar(scale);
    groupRef.current.add(model);

    // Initially hidden
    groupRef.current.visible = false;
    groupRef.current.scale.setScalar(0.5);

    const ctx = gsap.context(() => {
      // Make visible at the right scroll point
      ScrollTrigger.create({
        trigger: '.teardown-section',
        start: '30% top',
        end: '40% top',
        onEnter: () => {
          if (groupRef.current) groupRef.current.visible = true;
        },
        onLeaveBack: () => {
          if (groupRef.current) groupRef.current.visible = false;
        },
      });

      // Scale up using proxy
      const scaleProxy = { val: 0.5 };
      gsap.to(scaleProxy, {
        val: 1,
        scrollTrigger: {
          trigger: '.teardown-section',
          start: '30% top',
          end: '60% bottom',
          scrub: 1,
          onUpdate: () => {
            if (groupRef.current) {
              groupRef.current.scale.setScalar(scaleProxy.val);
            }
          },
        },
      });

      // Rotate slowly
      gsap.to(groupRef.current!.rotation, {
        y: Math.PI,
        scrollTrigger: {
          trigger: '.teardown-section',
          start: '30% top',
          end: 'bottom bottom',
          scrub: 1,
        },
      });
    });

    return () => ctx.revert();
  }, [scene]);

  return <group ref={groupRef} />;
}

// ── Main component ──────────────────────────────────────────────────────────

export function NotebookTeardown() {
  const sectionRef = useRef<HTMLElement>(null);
  const pinRef = useRef<HTMLDivElement>(null);
  const progressRef = useRef(0);
  const progressTextRef = useRef<HTMLDivElement>(null);

  useGSAP(
    () => {
      // Pin the stage and scrub through the animation
      ScrollTrigger.create({
        trigger: sectionRef.current,
        start: 'top top',
        end: 'bottom bottom',
        pin: pinRef.current,
        pinSpacing: false,
        onUpdate: (self) => {
          progressRef.current = self.progress;
          if (progressTextRef.current) {
            const pct = Math.round(self.progress * 100);
            progressTextRef.current.innerHTML = `<span>${pct}%</span> · SCROLL TO DISASSEMBLE`;
          }
        },
      });

      // Animate labels appearing at different scroll points
      const labels = gsap.utils.toArray<HTMLElement>('.teardown-label');
      labels.forEach((label, i) => {
        const startProgress = 0.15 + i * 0.12;
        gsap.fromTo(
          label,
          { opacity: 0, y: 20 },
          {
            opacity: 1,
            y: 0,
            scrollTrigger: {
              trigger: sectionRef.current,
              start: `start+=${startProgress * 100}% top`,
              end: `start+=${(startProgress + 0.05) * 100}% top`,
              scrub: 1,
            },
          },
        );
      });

      // Refresh after everything loads
      const refreshTimer = setTimeout(() => ScrollTrigger.refresh(), 500);
      return () => clearTimeout(refreshTimer);
    },
    { scope: sectionRef },
  );

  return (
    <section ref={sectionRef} className="teardown-section">
      <div ref={pinRef} className="teardown-pin">
        <div className="teardown-stage">
          <div className="teardown-canvas-container">
            <Canvas
              camera={{ position: [0, 0, 8], fov: 35 }}
              dpr={[1, 2]}
              gl={{ antialias: true, alpha: true, powerPreference: 'high-performance' }}
            >
              {/* eslint-disable react/no-unknown-property */}
              <ambientLight intensity={0.3} />
              <spotLight
                position={[10, 10, 10]}
                angle={0.15}
                penumbra={1}
                intensity={2}
                castShadow
              />
              <spotLight
                position={[-10, -5, 5]}
                angle={0.3}
                penumbra={1}
                intensity={1}
                color="#6c5ce7"
              />
              <Suspense fallback={null}>
                <Float speed={1.5} rotationIntensity={0.3} floatIntensity={0.5}>
                  <LaptopModel progressRef={progressRef} />
                </Float>
                <MotherboardModel />
                <Environment preset="city" />
                {}
                <ContactShadows position={[0, -2.5, 0]} opacity={0.4} scale={10} blur={2} far={4} />
              </Suspense>
            </Canvas>
          </div>

          <div className="teardown-overlay">
            <div className="teardown-label label-tl">CPU Performance</div>
            <div className="teardown-label label-tr">Thermal Control</div>
            <div className="teardown-label label-bl">Battery Health</div>
            <div className="teardown-label label-br">Fan Speed</div>
          </div>

          <div ref={progressTextRef} className="teardown-progress-text">
            <span>0%</span> · SCROLL TO DISASSEMBLE
          </div>
        </div>
      </div>
    </section>
  );
}

// Preload models
useGLTF.preload(`${import.meta.env.BASE_URL}landing/models/laptop.glb`);
useGLTF.preload(`${import.meta.env.BASE_URL}landing/models/motherboard.glb`);
