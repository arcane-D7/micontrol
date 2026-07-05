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
    desc: 'Switch between Balanced, Performance, and Turbo modes with a single click. Custom TDP profiles per mode.',
  },
  {
    icon: Thermometer,
    title: 'Thermal Monitoring',
    desc: 'Real-time CPU, GPU, and SSD temperature tracking with historical graphs and threshold alerts.',
  },
  {
    icon: Fan,
    title: 'Fan Control',
    desc: 'Custom fan curves with silent mode support. Set minimum RPM, maximum temperature, and ramp-up speed.',
  },
  {
    icon: Battery,
    title: 'Battery Insights',
    desc: 'Detailed battery health, wear level, charge cycles, and estimated time remaining at current usage.',
  },
  {
    icon: Monitor,
    title: 'Display Control',
    desc: 'Brightness, refresh rate, and color profile management. Automatic adjustment based on ambient light.',
  },
  {
    icon: Lock,
    title: 'Privacy First',
    desc: 'No telemetry, no cloud sync, no data collection. Everything runs locally on your machine.',
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
    <section ref={sectionRef} className="features-section" id="features">
      <div className="features-header">
        <span className="lp-section-tag">Features</span>
        <h2>
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
