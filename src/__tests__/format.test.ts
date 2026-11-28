import { describe, it, expect } from 'vitest';
import {
  formatCelsius,
  formatDecimal,
  formatDuration,
  formatVoltage,
  formatWatts,
  formatWh,
} from '../lib/format';

const NBSP = '\u00a0';

describe('formatação pt-PT (spec §5.7)', () => {
  it('usa vírgula decimal — nunca 17.28 V', () => {
    expect(formatVoltage(17280)).toBe(`17,28${NBSP}V`);
    expect(formatVoltage(17662)).toBe(`17,66${NBSP}V`);
    expect(formatVoltage(17280)).not.toContain('.');
  });

  it('separa número e unidade com espaço não separável', () => {
    const v = formatVoltage(17280);
    expect(v).toContain(NBSP);
    expect(v).not.toContain(' V');
  });

  it('sem leitura (0 / null) devolve — e não um zero falso', () => {
    expect(formatVoltage(0)).toBe('—');
    expect(formatVoltage(null)).toBe('—');
    expect(formatWatts(0)).toBe('—');
    expect(formatWh(null)).toBe('—');
  });

  it('watts inteiros e Wh com uma casa', () => {
    expect(formatWatts(65000)).toBe(`65${NBSP}W`);
    expect(formatWh(68200)).toBe(`68,2${NBSP}Wh`);
    expect(formatCelsius(31.4)).toBe(`31,4${NBSP}°C`);
  });

  it('durações legíveis', () => {
    expect(formatDuration(80)).toBe('1 h 20 min');
    expect(formatDuration(45)).toBe('45 min');
    expect(formatDuration(120)).toBe('2 h');
    expect(formatDuration(null)).toBe('');
    expect(formatDuration(-1)).toBe('');
  });

  it('formatDecimal respeita o número de casas', () => {
    expect(formatDecimal(1.5, 1)).toBe('1,5');
    expect(formatDecimal(1.25, 2)).toBe('1,25');
    expect(formatDecimal(1.4, 0)).toBe('1');
  });
});
