/**
 * Dados do resumo (tab Overview) que nenhuma outra tab já tem.
 *
 * O `useHardware` continua a ser a fonte do que é *vivo e partilhado* (uso de
 * CPU/GPU, temperaturas, bateria, ecrã). Este hook acrescenta o que só o
 * resumo mostra — identidade da máquina, especificações, NPU, memória, disco —
 * mais três leituras baratas de contexto (OS Turbo, WiFi, IoT).
 *
 * Cadência: uma leitura por montagem e um refrescamento a cada 60 s. O comando
 * de trás (`get_overview_snapshot`) devolve as especificações de um `OnceLock`
 * — a chamada repetida só relê espaço livre e o contador de uptime, por isso
 * 60 s não é polling agressivo; já o `get_iot_device_info` custa ~3,5 s na
 * ponte do IoT e é lido **uma única vez**, fora do caminho crítico.
 *
 * Cada leitura falha em silêncio para `null`: o cartão mostra `—` e continua a
 * mostrar o resto, em vez de derrubar o resumo inteiro.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IotDeviceInfo, OverviewSnapshot } from '../types/hardware';

/** `get_os_turbo` — só os campos que o resumo desenha. */
export interface OsTurboSummary {
  enabled: boolean;
  throttled_processes: number;
  power_plan: string;
}

/** `wifi_status`. */
export interface WifiSummary {
  connected: boolean;
  ssid: string | null;
  signal: number | null;
  interface: string | null;
}

/** `get_health_status` — estado dos serviços internos. */
export interface ServiceHealthSummary {
  bridge: { state: string } | null;
  iot: { state: string } | null;
  face: { state: string } | null;
  ambientSensor: { state: string } | null;
}

export interface OverviewData {
  snapshot: OverviewSnapshot | null;
  osTurbo: OsTurboSummary | null;
  wifi: WifiSummary | null;
  iot: IotDeviceInfo | null;
  services: ServiceHealthSummary | null;
  /** True while the first snapshot request is in flight. */
  loading: boolean;
  /** Re-reads everything that can change (not the IoT identity). */
  refresh: () => Promise<void>;
}

const SNAPSHOT_REFRESH_MS = 60_000;

export function useOverview(): OverviewData {
  const [snapshot, setSnapshot] = useState<OverviewSnapshot | null>(null);
  const [osTurbo, setOsTurbo] = useState<OsTurboSummary | null>(null);
  const [wifi, setWifi] = useState<WifiSummary | null>(null);
  const [iot, setIot] = useState<IotDeviceInfo | null>(null);
  const [services, setServices] = useState<ServiceHealthSummary | null>(null);
  const [loading, setLoading] = useState(true);

  const mounted = useRef(true);
  const inFlight = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  /** One snapshot read. Coalesced: a refresh during a read is dropped. */
  const loadSnapshot = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const next = await invoke<OverviewSnapshot>('get_overview_snapshot');
      if (mounted.current) setSnapshot(next);
    } catch (e) {
      console.warn('[overview] get_overview_snapshot failed:', e);
    } finally {
      inFlight.current = false;
      if (mounted.current) setLoading(false);
    }
  }, []);

  const loadContext = useCallback(async () => {
    const [turbo, wifiStatus, health] = await Promise.all([
      invoke<OsTurboSummary>('get_os_turbo').catch(() => null),
      invoke<WifiSummary>('wifi_status').catch(() => null),
      invoke<Record<string, { state?: string }>>('get_health_status').catch(() => null),
    ]);
    if (!mounted.current) return;
    setOsTurbo(turbo);
    setWifi(wifiStatus);
    if (health) {
      setServices({
        bridge: health.bridge ? { state: health.bridge.state ?? 'unknown' } : null,
        iot: health.iot ? { state: health.iot.state ?? 'unknown' } : null,
        face: health.face ? { state: health.face.state ?? 'unknown' } : null,
        ambientSensor: health.ambient_sensor
          ? { state: health.ambient_sensor.state ?? 'unknown' }
          : null,
      });
    } else {
      setServices(null);
    }
  }, []);

  // IoT identity: ~3.5 s over the pipe, so it never shares the critical path.
  const loadIot = useCallback(async () => {
    try {
      const info = await invoke<IotDeviceInfo>('get_iot_device_info');
      if (mounted.current) setIot(info);
    } catch (e) {
      console.warn('[overview] get_iot_device_info failed:', e);
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
    void loadContext();
    void loadIot();
  }, [loadSnapshot, loadContext, loadIot]);

  // Free space and uptime drift; the specs do not. 60 s is enough for a summary.
  useEffect(() => {
    const id = setInterval(() => {
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return;
      void loadSnapshot();
    }, SNAPSHOT_REFRESH_MS);
    return () => clearInterval(id);
  }, [loadSnapshot]);

  const refresh = useCallback(async () => {
    await Promise.all([loadSnapshot(), loadContext()]);
  }, [loadSnapshot, loadContext]);

  return { snapshot, osTurbo, wifi, iot, services, loading, refresh };
}
