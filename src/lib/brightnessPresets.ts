import type { AiBrightnessConfig } from '../types/hardware';
import type { IconName } from '../components/ui/icons';
import type { StringKey } from '../hooks/useI18n';

export interface BrightnessPreset {
  key: string;
  labelKey: StringKey;
  icon: IconName;
  /** Description hint key shown on hover */
  hintKey: StringKey;
  config: Omit<AiBrightnessConfig, 'enabled'>;
}

export const BRIGHTNESS_PRESETS: BrightnessPreset[] = [
  {
    key: 'eco',
    labelKey: 'brightness.presets.eco.label',
    icon: 'leaf',
    hintKey: 'brightness.presets.eco.hint',
    config: {
      min_brightness: 15,
      max_brightness: 75,
      sensitivity: 50,
      smoothing: 70,
      mode: 'curve',
    },
  },
  {
    key: 'padrao',
    labelKey: 'brightness.presets.standard.label',
    icon: 'building',
    hintKey: 'brightness.presets.standard.hint',
    config: {
      min_brightness: 10,
      max_brightness: 100,
      sensitivity: 100,
      smoothing: 30,
      mode: 'curve',
    },
  },
  {
    key: 'vivido',
    labelKey: 'brightness.presets.vivid.label',
    icon: 'bolt',
    hintKey: 'brightness.presets.vivid.hint',
    config: {
      min_brightness: 5,
      max_brightness: 100,
      sensitivity: 170,
      smoothing: 10,
      mode: 'curve',
    },
  },
];

/** Key of the preset that matches factory-default registry values. */
export const DEFAULT_PRESET_KEY = 'padrao';

/** Returns the preset key whose config exactly matches `cfg`, or null if custom. */
export function getActivePreset(
  cfg: Omit<AiBrightnessConfig, 'enabled'> | undefined | null,
): string | null {
  if (!cfg) return null;
  for (const p of BRIGHTNESS_PRESETS) {
    if (
      p.config.min_brightness === cfg.min_brightness &&
      p.config.max_brightness === cfg.max_brightness &&
      p.config.sensitivity === cfg.sensitivity &&
      p.config.smoothing === cfg.smoothing &&
      p.config.mode === cfg.mode
    ) {
      return p.key;
    }
  }
  return null;
}

export function getDefaultPresetConfig(): Omit<AiBrightnessConfig, 'enabled'> {
  return BRIGHTNESS_PRESETS.find((p) => p.key === DEFAULT_PRESET_KEY)!.config;
}
