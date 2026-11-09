import { describe, it, expect } from 'vitest';
import en from '../i18n/en.json';
import pt from '../i18n/pt.json';
import es from '../i18n/es.json';
import fr from '../i18n/fr.json';

const LOCALES = { pt, es, fr } as const;

function flatten(obj: unknown, prefix = ''): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === 'object') Object.assign(out, flatten(value, path));
    else out[path] = String(value);
  }
  return out;
}

const EN = flatten(en);
const placeholders = (s: string) => (s.match(/\{[A-Za-z0-9_]+\}/g) ?? []).sort();

describe('i18n locale parity', () => {
  for (const [code, locale] of Object.entries(LOCALES)) {
    const dict = flatten(locale);

    it(`${code}: has exactly the English key set`, () => {
      expect(Object.keys(dict).sort()).toEqual(Object.keys(EN).sort());
    });

    it(`${code}: has no empty values`, () => {
      const empty = Object.entries(dict)
        .filter(([, v]) => v.trim() === '')
        .map(([k]) => k);
      expect(empty).toEqual([]);
    });

    it(`${code}: keeps every placeholder of the English string`, () => {
      const mismatched = Object.keys(EN).filter((key) => {
        if (!(key in dict)) return false;
        return placeholders(EN[key]).join('|') !== placeholders(dict[key]).join('|');
      });
      expect(mismatched).toEqual([]);
    });
  }
});

describe('i18n coverage of the Face Unlock and IoT surfaces', () => {
  it('translates the Face Unlock wizard copy', () => {
    for (const dict of [EN, flatten(pt)]) {
      for (const key of [
        'face.title',
        'face.enroll.title',
        'face.enroll.accountQuestion',
        'face.settings.group.matching',
        'face.maint.confirmRemove',
      ]) {
        expect(dict[key], key).toBeTruthy();
      }
    }
  });

  it('translates the IoT device and module copy', () => {
    for (const dict of [EN, flatten(fr)]) {
      for (const key of [
        'iot.device.title',
        'iot.device.cloudBinding',
        'iot.device.protocol',
        'iot.cmd.bindStatus',
        'iot.module.rawRead',
      ]) {
        expect(dict[key], key).toBeTruthy();
      }
    }
  });
});
