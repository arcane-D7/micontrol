/**
 * One `<h1>` per tab, one `<h2>` per card/section.
 *
 * Before this component rendered a `<div>`, which is why the whole app had
 * zero `<h1>` and zero `<h2>` (audit 2026-11-07, P1-1 / WCAG 1.3.1 + 2.4.6):
 * screen-reader users navigating by heading got an empty outline, and the
 * partial `<h3>`s inside cards started the outline at level 3.
 *
 * The visual styling is unchanged — `.page-title` carries it, so the heading
 * level is pure structure.
 */
export function PageHeader({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <header className="page-header">
      <h1 className="page-title">{title}</h1>
      {subtitle && <p className="page-subtitle">{subtitle}</p>}
    </header>
  );
}
