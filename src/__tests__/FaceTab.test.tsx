import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import FaceUnlockTab from '../pages/tabs/face';

// Mock Tauri APIs used by the face tab.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

// Keep the small presentational helpers real-ish but avoid heavy deps.
vi.mock('../components/InfoModal', () => ({
  default: ({
    title,
    open,
    children,
  }: {
    title: string;
    open: boolean;
    children: React.ReactNode;
  }) =>
    open ? (
      <div role="dialog">
        <div>{title}</div>
        {children}
      </div>
    ) : null,
}));

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const STATUS_READY = {
  service_installed: true,
  service_running: true,
  pipe_available: true,
  enrolled_profiles: 1,
  camera_available: true,
};

const SETTINGS = {
  match_threshold: 0.4,
  match_margin: 0.05,
  liveness_enabled: true,
  antispoof_enabled: true,
  antispoof_threshold: 0.55,
  antispoof_max_frames: 10,
  lockout_max_fails: 5,
  lockout_seconds: 30,
  multi_face_protection_enabled: false,
  face_unlock_enabled: true,
  face_unlock_logon_enabled: true,
  face_unlock_workstation_enabled: true,
  renew_days: 60,
  language: 'en',
};

function mockCurrentState() {
  const inv = invoke as unknown as ReturnType<typeof vi.fn>;
  inv.mockReset();
  inv.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'face_status':
        return Promise.resolve(STATUS_READY);
      case 'face_get_settings':
        return Promise.resolve(SETTINGS);
      case 'face_models_status':
        return Promise.resolve({
          installed: true,
          staged: false,
          installed_dir: 'C:\\face',
          staging_dir: '',
          url: 'https://example.test/models',
        });
      case 'face_diagnostics':
        return Promise.resolve({
          camera: true,
          service: true,
          models: true,
          models_dir: 'C:\\face',
          data_dir: 'C:\\data',
        });
      case 'face_list_templates':
        return Promise.resolve({
          profiles: [{ name: 'alice', templates: 1, labels: ['front'] }],
        });
      default:
        return Promise.resolve({ ok: true });
    }
  });
  (listen as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(() => {});
}

describe('FaceUnlockTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCurrentState();
  });

  it('renders the ready state with enrolled face summary', async () => {
    render(<FaceUnlockTab />);

    // The hero title says the journey is ready and how many faces are enrolled.
    expect(await screen.findByText(/Face Unlock is ready · 1 face enrolled/)).toBeInTheDocument();
    expect(screen.getByText(/Everything is healthy/)).toBeInTheDocument();
  });

  it('shows advanced options when expanded', async () => {
    render(<FaceUnlockTab />);
    await screen.findByText(/Face Unlock is ready · 1 face enrolled/);

    fireEvent.click(screen.getByText('Advanced options'));
    expect(screen.getByText('Match margin (anti-misrouting)')).toBeInTheDocument();
    expect(screen.getByText('Anti-spoof threshold')).toBeInTheDocument();
    // Collapse again.
    fireEvent.click(screen.getByText('Hide advanced options'));
    expect(screen.queryByText('Match margin (anti-misrouting)')).not.toBeInTheDocument();
  });

  it('shows maintenance actions when expanded', async () => {
    render(<FaceUnlockTab />);
    await screen.findByText(/Face Unlock is ready · 1 face enrolled/);

    fireEvent.click(screen.getByText('Maintenance'));
    expect(screen.getByText('Run diagnostics')).toBeInTheDocument();
    expect(screen.getByText('Re-scan models')).toBeInTheDocument();
    expect(screen.getByText('Remove all models')).toBeInTheDocument();
    expect(screen.getByText('Reinstall / restart auth service')).toBeInTheDocument();
  });

  it('renders threat + sign-in screen labels', async () => {
    render(<FaceUnlockTab />);
    await screen.findByText(/Face Unlock is ready · 1 face enrolled/);

    expect(screen.getByText('Show tile at sign-in')).toBeInTheDocument();
    expect(screen.getByText('Show tile at lock (Win+L)')).toBeInTheDocument();
    expect(screen.getByText('Require liveness (blink / turn)')).toBeInTheDocument();
    expect(screen.getByText('Reject photos / videos (anti-spoof)')).toBeInTheDocument();
    expect(screen.getByText('Re-enrollment reminder')).toBeInTheDocument();
    expect(screen.getByText('Failed attempts before lockout')).toBeInTheDocument();
    expect(screen.getByText(/every 60 days/i)).toBeInTheDocument();
  });
});
