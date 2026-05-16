import type { AiBrightnessConfig } from "../hooks/useHardware";

export interface BrightnessPreset {
  key: string;
  label: string;
  icon: string;
  /** Description hint shown on hover */
  hint: string;
  config: Omit<AiBrightnessConfig, "enabled">;
}

export const BRIGHTNESS_PRESETS: BrightnessPreset[] = [
  {
    key: "eco",
    label: "Eco",
    icon: "🌿",
    hint: "Gentle transitions, capped brightness — saves battery",
    config: { min_brightness: 15, max_brightness: 75, sensitivity: 50, smoothing: 70 },
  },
  {
    key: "padrao",
    label: "Standard",
    icon: "🏙️",
    hint: "Balanced reactivity and smooth transitions",
    config: { min_brightness: 10, max_brightness: 100, sensitivity: 100, smoothing: 30 },
  },
  {
    key: "vivido",
    label: "Vivid",
    icon: "⚡",
    hint: "Fast, aggressive adaptation across full brightness range",
    config: { min_brightness: 5, max_brightness: 100, sensitivity: 170, smoothing: 10 },
  },
];

/** Key of the preset that matches factory-default registry values. */
export const DEFAULT_PRESET_KEY = "padrao";

/** Returns the preset key whose config exactly matches `cfg`, or null if custom. */
export function getActivePreset(
  cfg: Omit<AiBrightnessConfig, "enabled"> | undefined | null
): string | null {
  if (!cfg) return null;
  for (const p of BRIGHTNESS_PRESETS) {
    if (
      p.config.min_brightness === cfg.min_brightness &&
      p.config.max_brightness === cfg.max_brightness &&
      p.config.sensitivity === cfg.sensitivity &&
      p.config.smoothing === cfg.smoothing
    ) {
      return p.key;
    }
  }
  return null;
}

export function getDefaultPresetConfig(): Omit<AiBrightnessConfig, "enabled"> {
  return BRIGHTNESS_PRESETS.find((p) => p.key === DEFAULT_PRESET_KEY)!.config;
}
