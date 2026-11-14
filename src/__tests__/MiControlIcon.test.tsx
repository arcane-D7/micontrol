import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { MiControlIcon } from '../components/MiControlIcon';

describe('MiControlIcon', () => {
  it('renders the brand mark as a single currentColor path', () => {
    const { container } = render(<MiControlIcon />);
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('viewBox', '-2 -2 82 76');
    expect(svg).toHaveAttribute('aria-hidden', 'true');

    const paths = svg.querySelectorAll('path');
    expect(paths).toHaveLength(1);
    expect(paths[0]).toHaveAttribute('fill', 'currentColor');
    expect(paths[0].getAttribute('d')).toMatch(/^M0 0L17 0L39 38\.105/);
  });

  it('carries no accent colour (brand palette is closed)', () => {
    const { container } = render(<MiControlIcon />);
    expect(container.innerHTML).not.toContain('var(--accent)');
    expect(container.querySelectorAll('circle, line, polyline')).toHaveLength(0);
  });

  it('scales the width from size and keeps the mark ratio', () => {
    const { container } = render(<MiControlIcon size={16} />);
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('width', '16');
    // 16 × 76/82 ≈ 14.83
    expect(Number(svg.getAttribute('height'))).toBeCloseTo(14.83, 2);
  });

  it('keeps caller overrides (className, style, aria)', () => {
    const { container } = render(
      <MiControlIcon size={24} className="brand" style={{ marginTop: 2 }} aria-hidden={false} />,
    );
    const svg = container.querySelector('svg')!;
    expect(svg).toHaveClass('brand');
    expect(svg).toHaveStyle({ marginTop: '2px', flexShrink: '0' });
    expect(svg).toHaveAttribute('aria-hidden', 'false');
  });
});
