import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';

const APP_VERSION = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0';

export default function AboutTab() {
  return (
    <>
      <PageHeader title={t('about.title')} />
      <div className="card">
        <div className="grid-2">
          <div>
            <div className="stat-row">
              <span className="stat-label">{t('about.appName')}</span>
              <span className="stat-value">MiControl</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.version')}</span>
              <span className="stat-value">{APP_VERSION}</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.device')}</span>
              <span className="stat-value">Xiaomi Laptop Pro</span>
            </div>
          </div>
          <div>
            <div className="stat-row">
              <span className="stat-label">{t('about.author')}</span>
              <span className="stat-value">MiControl Contributors</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.license')}</span>
              <span className="stat-value">MIT License</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.github')}</span>
              <span className="stat-value">GitHub Repository</span>
            </div>
          </div>
        </div>
        <p style={{ marginTop: 16, fontSize: 12, color: 'var(--color-text-muted)' }}>
          {t('about.description')}
        </p>
      </div>
      <div className="card">
        <div className="card-title">{t('about.drivers')}</div>
        <div className="grid-2">
          <div className="stat-row">
            <span className="stat-label">{t('about.driversList.virtualControlHID')}</span>
          </div>
          <div className="stat-row">
            <span className="stat-label">{t('about.driversList.iotDriver')}</span>
          </div>
        </div>
      </div>
    </>
  );
}
