import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, act, within } from '@testing-library/react';
import ColorTab from '../pages/tabs/color';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

const OEM_ENTRY = {
  id: 'oem-edo4503-d65p3',
  display_name: 'OLED D65 P3 base (OEM)',
  description: 'Maps the panel to Display P3 primaries at D65.',
  source: 'OEM (Xiaomi)',
  source_url: null,
  license_note: 'Shipped by the OEM.',
  kind: 'local',
  color_space: 'Display P3 (D65)',
  sha256: null,
  sha256_reference: '648344e52bf63283ce464de45c51f34537cb35cbacd9fd4d27807e0afb8e0e62',
  bytes: 568,
  file_name: 'xiaomi_D65P3_base.icc',
  available_locally: true,
  match_quality: 'exact',
  compatible: true,
};

const SRGB_ENTRY = {
  id: 'windows-srgb-iec61966-2-1',
  display_name: 'sRGB IEC61966-2.1',
  description: 'The Windows reference sRGB profile.',
  source: 'Windows',
  source_url: 'https://learn.microsoft.com/windows/win32/wcs/standard-color-profiles',
  license_note: 'Microsoft Windows standard profile.',
  kind: 'local',
  color_space: 'sRGB',
  sha256: null,
  sha256_reference: '2b3aa1645779a9e634744faf9b01e9102b0c9b88fd6deced7934df86b949af7e',
  bytes: 3144,
  file_name: 'sRGB Color Space Profile.icm',
  available_locally: true,
  match_quality: 'generic',
  compatible: false,
};

const ICC_DOWNLOAD_ENTRY = {
  id: 'icc-srgb2014',
  display_name: 'sRGB v4 (ICC, 2014)',
  description: "The ICC's current sRGB v4 display profile.",
  source: 'International Color Consortium',
  source_url: 'https://registry.color.org/rgb-registry/profiles/sRGB2014.icc',
  license_note: 'ICC copyright.',
  kind: 'download',
  color_space: 'sRGB',
  sha256: '384b832de3412066743b52a75ee906b6fb9fb8d9e09e936fc2c43223815c6e0a',
  sha256_reference: null,
  bytes: 3024,
  file_name: 'MiControl-sRGB2014.icc',
  available_locally: false,
  match_quality: 'generic',
  compatible: false,
};

function statusWith(overrides: Record<string, unknown> = {}) {
  return {
    displays: [
      {
        device_name: '\\\\.\\DISPLAY1',
        label: '',
        current_profile: null,
        installed_profiles: [
          'C:\\Windows\\System32\\spool\\drivers\\color\\sRGB Color Space Profile.icm',
        ],
        advanced_color_supported: true,
        hdr_enabled: false,
        panel: {
          panel_id: 'EDO 0x4503',
          manufacturer: 'EDO',
          product_code: '4503',
          name: 'EDO 0x4503',
          model_token: 'EDO4503',
          year: 2025,
        },
        catalog: [OEM_ENTRY, SRGB_ENTRY, ICC_DOWNLOAD_ENTRY],
        ...overrides,
      },
    ],
    eye_protection_active: false,
    gamma_intensity: 0,
    wizard_target: '\\\\.\\DISPLAY1',
    wizard_block: null,
    catalog_updated: '2026-11-09',
  };
}

async function renderTab(status: unknown) {
  mockedInvoke.mockImplementation((cmd: string) => {
    if (cmd === 'get_color_status') return Promise.resolve(status);
    if (cmd === 'get_eye_protection') return Promise.resolve({ enabled: false, intensity: 50 });
    if (cmd === 'download_color_profile') {
      return Promise.resolve({
        entry_id: 'icc-srgb2014',
        file_name: 'MiControl-sRGB2014.icc',
        staging_path: 'C:\\ProgramData\\MiControl\\icc\\downloads\\MiControl-sRGB2014.icc',
        sha256: '384b832de3412066743b52a75ee906b6fb9fb8d9e09e936fc2c43223815c6e0a',
        bytes: 3024,
        profile_class: 'mntr',
        color_space: 'RGB',
      });
    }
    return Promise.resolve({});
  });
  await act(async () => {
    render(<ColorTab />);
  });
}

function entryCard(name: string): HTMLElement {
  const card = screen.getByText(name).closest('.color-entry');
  if (!card) throw new Error(`no catalogue row for ${name}`);
  return card as HTMLElement;
}

describe('ColorTab — monitor profile catalogue', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the EDID panel and marks panel-specific profiles as compatible', async () => {
    await renderTab(statusWith());
    expect(screen.getAllByText('EDO 0x4503').length).toBeGreaterThan(0);
    expect(screen.getByText('2026-11-09', { exact: false })).toBeInTheDocument();

    const oem = entryCard('OLED D65 P3 base (OEM)');
    expect(within(oem).getByText('Compatible')).toBeInTheDocument();
    const srgb = entryCard('sRGB IEC61966-2.1');
    expect(within(srgb).getByText('Generic')).toBeInTheDocument();
    expect(within(srgb).queryByText('Compatible')).not.toBeInTheDocument();
  });

  it('degrades to an em dash when Windows reports no EDID identity', async () => {
    await renderTab(statusWith({ panel: null, catalog: [SRGB_ENTRY] }));
    expect(screen.getByText('—')).toBeInTheDocument();
    expect(screen.getByText(/Windows did not report an EDID identity/)).toBeInTheDocument();
    expect(screen.queryByText('OLED D65 P3 base (OEM)')).not.toBeInTheDocument();
  });

  it('warns and disables Apply while HDR is on', async () => {
    await renderTab(statusWith({ hdr_enabled: true }));
    expect(screen.getByText(/HDR is on — Windows ignores ICC profiles/)).toBeInTheDocument();
    const oem = entryCard('OLED D65 P3 base (OEM)');
    expect(within(oem).getByRole('button', { name: 'Apply' })).toBeDisabled();
  });

  it('downloads, validates and only then offers Install and apply', async () => {
    await renderTab(statusWith());
    const entry = entryCard('sRGB v4 (ICC, 2014)');
    expect(within(entry).getByRole('button', { name: 'Download and validate' })).toBeEnabled();

    await act(async () => {
      within(entry).getByRole('button', { name: 'Download and validate' }).click();
    });

    await waitFor(() =>
      expect(within(entry).getByRole('button', { name: 'Install and apply' })).toBeInTheDocument(),
    );
    expect(mockedInvoke).toHaveBeenCalledWith('download_color_profile', {
      entryId: 'icc-srgb2014',
    });

    await act(async () => {
      within(entry).getByRole('button', { name: 'Install and apply' }).click();
    });
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith('apply_color_profile', {
        display: '\\\\.\\DISPLAY1',
        entryId: 'icc-srgb2014',
      }),
    );
  });

  it('opens real searches with the panel token', async () => {
    await renderTab(statusWith());
    await act(async () => {
      screen.getByRole('button', { name: 'Panelook' }).click();
    });
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith('open_color_profile_search', {
        kind: 'panelook',
        query: 'EDO4503',
      }),
    );
  });
});
