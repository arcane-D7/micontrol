import { t } from '../hooks/useI18n';

function PageHeader({ title }: { title: string }) {
  return (
    <div className="page-header">
      <div className="page-title">{title}</div>
    </div>
  );
}

export default function PrivacyPolicy() {
  return (
    <>
      <PageHeader title={t('privacy.title')} />
      <div className="card">
        <p style={{ fontSize: 12, color: 'var(--color-text-muted)', marginBottom: 20 }}>
          {t('privacy.lastUpdated')}
        </p>

        <Section title={t('privacy.section1Title')}>
          <p>{t('privacy.section1Body')}</p>
        </Section>

        <Section title={t('privacy.section2Title')}>
          <p>{t('privacy.section2Body')}</p>
        </Section>

        <Section title={t('privacy.section3Title')}>
          <p>{t('privacy.section3Body')}</p>
        </Section>

        <Section title={t('privacy.section4Title')}>
          <p>{t('privacy.section4Body')}</p>
        </Section>

        <Section title={t('privacy.section5Title')}>
          <p>{t('privacy.section5Body')}</p>
        </Section>

        <Section title={t('privacy.section6Title')}>
          <p>{t('privacy.section6Body')}</p>
        </Section>
      </div>
    </>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 20 }}>
      <h3
        style={{
          fontSize: 14,
          fontWeight: 700,
          marginBottom: 8,
          marginTop: 0,
          color: 'var(--color-text)',
        }}
      >
        {title}
      </h3>
      <div
        style={{
          fontSize: 13,
          lineHeight: 1.7,
          color: 'var(--color-text-muted)',
          whiteSpace: 'pre-line',
        }}
      >
        {children}
      </div>
    </div>
  );
}
