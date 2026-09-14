import { useRef } from 'react';
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { useGSAP } from '@gsap/react';
import { Gauge, Battery, Fan, Monitor, Lock, Thermometer } from 'lucide-react';

gsap.registerPlugin(ScrollTrigger, useGSAP);

const features = [
  {
    icon: Gauge,
    title: 'Performance Modes',
    desc: 'Switch between the performance profiles supported by your notebook and keep everyday tuning in one place.',
  },
  {
    icon: Thermometer,
    title: 'Thermal Monitoring',
    desc: 'Monitor the hardware readings exposed by the embedded controller, including temperatures and power information.',
  },
  {
    icon: Fan,
    title: 'Fan Control',
    desc: 'Read fan RPM in real time and choose the available cooling profile for the current workload.',
  },
  {
    icon: Battery,
    title: 'Battery Care',
    desc: 'Set a supported charge limit to reduce time spent at full charge during long plugged-in sessions.',
  },
  {
    icon: Monitor,
    title: 'Display Control',
    desc: 'Adjust brightness, HDR, adaptive refresh, and other display controls when they are supported by the hardware.',
  },
  {
    icon: Lock,
    title: 'Privacy First',
    desc: 'Control telemetry consent in the app settings. Hardware controls run locally, and the project source is available on GitHub.',
  },
];

export function FeaturesSection() {
  const sectionRef = useRef<HTMLElement>(null);

  useGSAP(
    () => {
      gsap.from('.feature-card', {
        y: 60,
        opacity: 0,
        duration: 0.6,
        stagger: 0.1,
        scrollTrigger: {
          trigger: '.features-grid',
          start: 'top 80%',
          end: 'bottom 60%',
          scrub: 1,
        },
      });

      gsap.from('.features-header > *', {
        y: 30,
        opacity: 0,
        duration: 0.6,
        stagger: 0.12,
        scrollTrigger: {
          trigger: '.features-header',
          start: 'top 80%',
          end: 'bottom 50%',
          scrub: 1,
        },
      });
    },
    { scope: sectionRef },
  );

  return (
    <section
      ref={sectionRef}
      className="features-section"
      id="features"
      aria-labelledby="features-title"
    >
      <div className="features-header">
        <span className="lp-section-tag">Features</span>
        <h2 id="features-title">
          Everything You Need,
          <br />
          Nothing You Don&apos;t
        </h2>
        <p>
          A comprehensive toolkit for your Xiaomi Notebook, built with the same attention to detail
          as the hardware it controls.
        </p>
      </div>
      <div className="features-grid">
        {features.map((feature) => {
          const Icon = feature.icon;
          return (
            <div key={feature.title} className="feature-card">
              <div className="feature-card-icon">
                <Icon size={24} />
              </div>
              <h3>{feature.title}</h3>
              <p>{feature.desc}</p>
            </div>
          );
        })}
      </div>
    </section>
  );
}
