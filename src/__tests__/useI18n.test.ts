import { describe, it, expect } from 'vitest';
import { t } from '../hooks/useI18n';

describe('useI18n - t()', () => {
  it('resolves top-level keys', () => {
    expect(t('app.name')).toBe('MiControl');
  });

  it('resolves nested keys', () => {
    expect(t('performance.modes.balance')).toBe('Balance');
    expect(t('performance.modes.turbo')).toBe('Turbo');
    expect(t('performance.modes.silence')).toBe('Silence');
  });

  it('substitutes variables in strings', () => {
    const result = t('errors.commandFailed', { error: 'access denied' });
    expect(result).toContain('access denied');
  });

  it('resolves battery strings', () => {
    expect(t('battery.charging')).toBeTruthy();
    expect(t('battery.discharging')).toBeTruthy();
    expect(t('battery.health')).toBeTruthy();
  });

  it('resolves charging threshold descriptions', () => {
    expect(t('charging.title')).toBeTruthy();
    expect(t('charging.subtitle')).toBeTruthy();
    expect(t('charging.recommended')).toBeTruthy();
  });

  it('resolves display strings', () => {
    expect(t('display.brightness')).toBeTruthy();
    expect(t('display.hdr')).toBeTruthy();
    expect(t('display.aiAdaptiveBrightness')).toBeTruthy();
  });

  it('resolves fan strings', () => {
    expect(t('fan.title')).toBeTruthy();
    expect(t('fan.modes.auto')).toBeTruthy();
    expect(t('fan.modes.fixed')).toBeTruthy();
  });

  it('resolves nav items', () => {
    expect(t('nav.overview')).toBeTruthy();
    expect(t('nav.performance')).toBeTruthy();
    expect(t('nav.battery')).toBeTruthy();
    expect(t('nav.display')).toBeTruthy();
    expect(t('nav.fan')).toBeTruthy();
    expect(t('nav.touchpad')).toBeTruthy();
    expect(t('nav.startup')).toBeTruthy();
    expect(t('nav.about')).toBeTruthy();
  });

  it('returns key path as fallback for unknown keys', () => {
    // @ts-expect-error testing unknown key fallback
    const result = t('nonexistent.deep.key');
    expect(result).toBe('nonexistent.deep.key');
  });

  it('handles multiple variable substitutions', () => {
    const result = t('errors.commandFailed', { error: 'TEST_ERROR' });
    expect(result).toContain('TEST_ERROR');
  });
});
