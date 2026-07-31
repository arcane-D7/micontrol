import { memo, useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import BatteryInfoCard from '../../components/BatteryInfo';
import ChargingThreshold from '../../components/ChargingThreshold';
import type { Hardware } from './shared';

interface Props {
  hw: Hardware;
}

function BatteryTab({ hw }: Props) {
  const [batteryCare, setBatteryCare] = useState<boolean | null>(null);
  const [careToggling, setCareToggling] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchBatteryCare = useCallback(async () => {
    try {
      const care = await invoke<boolean>('get_battery_care');
      setBatteryCare(care);
    } catch {
      setBatteryCare(null);
    }
  }, []);

  useEffect(() => {
    void fetchBatteryCare();
  }, [fetchBatteryCare]);

  const handleToggleBatteryCare = async () => {
    if (batteryCare === null) return;
    setCareToggling(true);
    try {
      await invoke('set_battery_care', { enabled: !batteryCare });
      setBatteryCare(!batteryCare);
    } catch (e) {
      setErrorMsg(String(e));
    }
    setCareToggling(false);
  };

  return (
    <>
      <PageHeader title={t('battery.title')} />

      {errorMsg && (
        <div className="alert alert-error" style={{ marginBottom: 16 }}>
          ⚠ {errorMsg}
        </div>
      )}

      <BatteryInfoCard battery={hw.battery} />
      <ChargingThreshold
        threshold={hw.chargingThreshold}
        onThresholdChange={hw.setChargingThreshold}
      />

      {/* Battery Care Card */}
      <div className="card" style={{ marginTop: 16 }}>
        <h3>🔋 {t('battery.batteryCare')}</h3>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 12 }}>
          {t('battery.batteryCareDesc')}
        </p>
        <label className="toggle-row" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <input
            type="checkbox"
            checked={batteryCare ?? false}
            onChange={handleToggleBatteryCare}
            disabled={careToggling || batteryCare === null}
          />
          <span>
            {batteryCare === null
              ? t('common.loading')
              : batteryCare
                ? t('battery.batteryCareOn')
                : t('battery.batteryCareOff')}
          </span>
        </label>
      </div>
    </>
  );
}
export default memo(BatteryTab);
