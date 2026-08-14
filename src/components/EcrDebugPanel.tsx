import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { useI18n } from '../hooks/useI18n';

/** Regex matching one or more hex digits (no whitespace, no 0x prefix). */
const HEX_RE = /^[0-9A-Fa-f]+$/;

/** Known-safe EC RAM physical address range (ERAM base through IoTDevice state block end). */
const EC_RAM_SAFE_MIN = 0xfe0b0300;
const EC_RAM_SAFE_MAX = 0xfe0b0f80;

/** Parse a hex address string (with or without 0x prefix) into a number, or null if invalid. */
function parseHexAddress(addr: string): number | null {
  const cleaned = addr.trim().replace(/^0[xX]/, '');
  if (!HEX_RE.test(cleaned)) return null;
  return parseInt(cleaned, 16);
}

/** Validate that a hex data string contains only hex characters (whitespace stripped). */
function isValidHexData(hex: string): boolean {
  const cleaned = hex.replace(/\s+/g, '');
  return cleaned.length > 0 && HEX_RE.test(cleaned);
}

export default function EcrDebugPanel() {
  const [hexData, setHexData] = useState('');
  const [address, setAddress] = useState('');
  const [count, setCount] = useState('32');
  const [result, setResult] = useState('');
  const [loading, setLoading] = useState(false);
  const { addToast } = useToast();
  const { t } = useI18n();

  const handleRead = async () => {
    setLoading(true);
    try {
      const data = await invoke<string>('read_ecram_raw', {
        address: address || '0x0',
        count: parseInt(count) || 32,
      });
      setResult(data);
    } catch (e) {
      addToast({
        message: t('ecrDebug.readError', { error: String(e) }),
        type: 'error',
        onRetry: handleRead,
      });
    } finally {
      setLoading(false);
    }
  };

  const handleWrite = async () => {
    if (!address || !hexData) return;

    // Validate hex data format
    if (!isValidHexData(hexData)) {
      addToast({
        message: t('ecrDebug.invalidHexData'),
        type: 'error',
      });
      return;
    }

    // Validate address format and range
    const addrNum = parseHexAddress(address);
    if (addrNum === null) {
      addToast({ message: t('ecrDebug.invalidAddress'), type: 'error' });
      return;
    }
    if (addrNum < EC_RAM_SAFE_MIN || addrNum >= EC_RAM_SAFE_MAX) {
      addToast({
        message: t('ecrDebug.addressOutOfRange', {
          min: EC_RAM_SAFE_MIN.toString(16).toUpperCase(),
          max: (EC_RAM_SAFE_MAX - 1).toString(16).toUpperCase(),
        }),
        type: 'error',
      });
      return;
    }

    // Confirmation dialog — writing to EC RAM can brick the device
    const confirmed = window.confirm(t('ecrDebug.writeWarning'));
    if (!confirmed) return;

    setLoading(true);
    try {
      await invoke('write_iot_hex', {
        address,
        hexData,
      });
      addToast({ message: t('ecrDebug.writeSuccess'), type: 'success' });
    } catch (e) {
      addToast({
        message: t('ecrDebug.writeError', { error: String(e) }),
        type: 'error',
        onRetry: handleWrite,
      });
    } finally {
      setLoading(false);
    }
  };

  const handleReadMap = async () => {
    setLoading(true);
    try {
      const map = await invoke<string>('get_ecram_map');
      setResult(JSON.stringify(map, null, 2));
    } catch (e) {
      addToast({
        message: t('ecrDebug.mapError', { error: String(e) }),
        type: 'error',
        onRetry: handleReadMap,
      });
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">{t('ecrDebug.title')}</div>
      <p className="page-subtitle">{t('ecrDebug.subtitle')}</p>

      {/* Read */}
      <div style={{ marginTop: 12 }}>
        <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: 'var(--text-dim)' }}>
          {t('ecrDebug.readEcram')}
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input
            className="text-input"
            type="text"
            placeholder={t('ecrDebug.addressPlaceholder')}
            aria-label={t('ecrDebug.addressPlaceholder')}
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            style={{ flex: 1, minWidth: 120 }}
          />
          <input
            className="text-input"
            type="number"
            placeholder={t('ecrDebug.countPlaceholder')}
            aria-label={t('ecrDebug.countPlaceholder')}
            value={count}
            onChange={(e) => setCount(e.target.value)}
            min={1}
            max={256}
            style={{ width: 80 }}
          />
          <button
            className="btn btn-primary"
            onClick={handleRead}
            disabled={loading}
            aria-label={t('ecrDebug.readButton')}
          >
            {t('ecrDebug.readButton')}
          </button>
        </div>
      </div>

      {/* Write */}
      <div style={{ marginTop: 12 }}>
        <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: 'var(--text-dim)' }}>
          {t('ecrDebug.writeEcram')}
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input
            className="text-input"
            type="text"
            placeholder={t('ecrDebug.addressPlaceholder')}
            aria-label={t('ecrDebug.addressPlaceholder')}
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            style={{ flex: 1, minWidth: 120 }}
          />
          <input
            className="text-input"
            type="text"
            placeholder={t('ecrDebug.hexDataPlaceholder')}
            aria-label={t('ecrDebug.hexDataPlaceholder')}
            value={hexData}
            onChange={(e) => setHexData(e.target.value)}
            style={{ flex: 2, minWidth: 160 }}
          />
          <button
            className="btn btn-primary"
            onClick={handleWrite}
            disabled={loading}
            aria-label={t('ecrDebug.writeButton')}
          >
            {t('ecrDebug.writeButton')}
          </button>
        </div>
      </div>

      {/* Read Map */}
      <button
        className="btn btn-secondary"
        onClick={handleReadMap}
        disabled={loading}
        aria-label={t('ecrDebug.readMap')}
        style={{ marginTop: 12, width: '100%' }}
      >
        {t('ecrDebug.readMap')}
      </button>

      {/* Result */}
      {result && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: 'var(--text-dim)' }}>
            {t('ecrDebug.result')}
          </div>
          <pre
            style={{
              padding: 12,
              background: 'var(--bg-hover)',
              borderRadius: 'var(--r-xs)',
              fontSize: 12,
              maxHeight: 300,
              overflow: 'auto',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-all',
            }}
          >
            {result}
          </pre>
        </div>
      )}
    </div>
  );
}
