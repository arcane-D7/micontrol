import { useRef, Suspense, useEffect, useState } from 'react';
import { Canvas } from '@react-three/fiber';
import { useGLTF, Environment, ContactShadows, Float, Html } from '@react-three/drei';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import * as THREE from 'three';
import { Cpu, Thermometer, Zap, Fan } from 'lucide-react';

gsap.registerPlugin(ScrollTrigger, useGSAP);

function ModelLoader({ label }: { label: string }) {
  return (
    <Html center>
      <div className="model-loader" role="status" aria-live="polite">
        <span className="model-loader-spinner" aria-hidden="true" />
        <span>{label}</span>
      </div>
    </Html>
  );
}

function GearModel() {
  const groupRef = useRef<THREE.Group>(null);
  const { scene } = useGLTF(`${import.meta.env.BASE_URL}landing/models/gear.glb`) as unknown as {
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
    // Flatten on Z axis to make the gear thinner
    model.scale.z *= 0.35;
    groupRef.current.add(model);

    const ctx = gsap.context(() => {
      // Scroll-driven rotation
      gsap.to(groupRef.current!.rotation, {
        y: Math.PI * 4,
        scrollTrigger: {
          trigger: '.gear-section',
          start: 'top bottom',
          end: 'bottom top',
          scrub: 1.5,
        },
      });

      // Float effect using proxy
      const posProxy = { y: 0 };
      gsap.to(posProxy, {
        y: 0.3,
        duration: 2,
        repeat: -1,
        yoyo: true,
        ease: 'sine.inOut',
        onUpdate: () => {
          if (groupRef.current) {
            groupRef.current.position.y = posProxy.y;
          }
        },
      });
    });

    return () => ctx.revert();
  }, [scene]);

  return <group ref={groupRef} />;
}

export function GearSection() {
  const sectionRef = useRef<HTMLElement>(null);
  const [isNearViewport, setIsNearViewport] = useState(false);

  useEffect(() => {
    const section = sectionRef.current;
    if (!section) return;

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
      { rootMargin: '600px 0px' },
    );
    observer.observe(section);

    return () => observer.disconnect();
  }, []);

  useGSAP(
    () => {
      // Animate content in
      gsap.from('.gear-content > *', {
        y: 40,
        opacity: 0,
        duration: 0.8,
        stagger: 0.15,
        scrollTrigger: {
          trigger: sectionRef.current,
          start: 'top 70%',
          end: 'top 30%',
          scrub: 1,
        },
      });

      // Animate feature items
      gsap.from('.gear-feature-item', {
        x: -20,
        opacity: 0,
        duration: 0.5,
        stagger: 0.1,
        scrollTrigger: {
          trigger: '.gear-feature-list',
          start: 'top 80%',
          end: 'bottom 60%',
          scrub: 1,
        },
      });
    },
    { scope: sectionRef },
  );

  return (
    <section ref={sectionRef} className="gear-section" aria-labelledby="gear-title">
      <div className="gear-stage">
        <div className="gear-visual">
          <Canvas
            camera={{ position: [0, 2, 6], fov: 40 }}
            dpr={[1, 2]}
            gl={{ antialias: true, alpha: true, powerPreference: 'high-performance' }}
          >
            {/* eslint-disable react/no-unknown-property */}
            <ambientLight intensity={0.3} />
            <spotLight position={[5, 10, 5]} angle={0.3} penumbra={1} intensity={2} castShadow />
            <spotLight
              position={[-5, 2, 3]}
              angle={0.3}
              penumbra={1}
              intensity={1}
              color="#6c5ce7"
            />
            {isNearViewport ? (
              <Suspense fallback={<ModelLoader label="Loading hardware view" />}>
                <Float speed={2} rotationIntensity={0.5} floatIntensity={0.8}>
                  <GearModel />
                </Float>
                <Environment preset="city" />
                <ContactShadows position={[0, -2, 0]} opacity={0.4} scale={8} blur={2} far={4} />
              </Suspense>
            ) : (
              <ModelLoader label="Hardware view loads as you scroll" />
            )}
          </Canvas>
        </div>

        <div className="gear-content">
          <span className="lp-section-tag">Hardware Control</span>
          <h2 id="gear-title">
            Every Component,
            <br />
            Under Your Command
          </h2>
          <p>
            miControl connects to the hardware interfaces available on your notebook, giving you
            focused control over performance, thermals, and power delivery from a single interface.
          </p>
          <ul className="gear-feature-list">
            <li className="gear-feature-item">
              <span className="gear-feature-icon">
                <Cpu size={16} />
              </span>
              <span>Performance mode switching (Balanced, Performance, Turbo)</span>
            </li>
            <li className="gear-feature-item">
              <span className="gear-feature-icon">
                <Thermometer size={16} />
              </span>
              <span>Real-time temperature monitoring with custom fan curves</span>
            </li>
            <li className="gear-feature-item">
              <span className="gear-feature-icon">
                <Zap size={16} />
              </span>
              <span>TDP control and power delivery optimization</span>
            </li>
            <li className="gear-feature-item">
              <span className="gear-feature-icon">
                <Fan size={16} />
              </span>
              <span>Fan speed override with silent mode support</span>
            </li>
          </ul>
        </div>
      </div>
    </section>
  );
}
