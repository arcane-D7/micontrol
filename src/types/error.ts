/**
 * Error response from the Rust backend.
 * Matches the `ErrorResponse` struct in `src-tauri/src/hw/errors.rs`.
 */
export interface ErrorResponse {
  code: string;
  message: string;
}

/**
 * Stable error codes matching the Rust `HardwareError::code()` method.
 * @see src-tauri/src/hw/errors.rs
 */
export const ERROR_CODES = {
  WMI_QUERY: 'WMI_QUERY',
  WMI_CONNECTION: 'WMI_CONNECTION',
  REGISTRY: 'REGISTRY',
  DEVICE_NOT_FOUND: 'DEVICE_NOT_FOUND',
  PERMISSION_DENIED: 'PERMISSION_DENIED',
  INVALID_ARGUMENT: 'INVALID_ARGUMENT',
  IO: 'IO',
  TIMEOUT: 'TIMEOUT',
  CRYPTO: 'CRYPTO',
  IPC: 'IPC',
  HARDWARE: 'HARDWARE',
  AI_CONSENT_DENIED: 'AI_CONSENT_DENIED',
  AI_REQUEST_FAILED: 'AI_REQUEST_FAILED',
  AI_RESPONSE_INVALID: 'AI_RESPONSE_INVALID',
  GENERIC: 'GENERIC',
} as const;

/**
 * Parse an error from a Tauri invoke rejection.
 * Tauri rejects with a string (the error message) or an object with code+message.
 */
export function parseErrorResponse(error: unknown): ErrorResponse {
  if (typeof error === 'string') {
    // Try to parse as JSON (ErrorResponse format)
    try {
      const parsed = JSON.parse(error);
      if (parsed.code && parsed.message) {
        return { code: parsed.code, message: parsed.message };
      }
    } catch {
      // Not JSON, treat as generic error
    }
    return { code: ERROR_CODES.GENERIC, message: error };
  }
  if (error && typeof error === 'object' && 'code' in error && 'message' in error) {
    return {
      code: String((error as ErrorResponse).code),
      message: String((error as ErrorResponse).message),
    };
  }
  return { code: ERROR_CODES.GENERIC, message: 'An unexpected error occurred' };
}

/**
 * Get a user-friendly error message based on the error code.
 */
export function getUserFriendlyMessage(error: ErrorResponse): string {
  switch (error.code) {
    case ERROR_CODES.WMI_QUERY:
    case ERROR_CODES.WMI_CONNECTION:
      return 'Hardware information is temporarily unavailable. The system may be busy.';
    case ERROR_CODES.DEVICE_NOT_FOUND:
      return 'The requested hardware device was not found.';
    case ERROR_CODES.PERMISSION_DENIED:
      return 'Permission denied. Administrator privileges may be required.';
    case ERROR_CODES.TIMEOUT:
      return 'The operation timed out. Please try again.';
    case ERROR_CODES.AI_CONSENT_DENIED:
      return 'AI analysis requires telemetry consent. Please grant consent in Settings.';
    case ERROR_CODES.AI_REQUEST_FAILED:
      return 'AI analysis failed. Please check your connection and try again.';
    case ERROR_CODES.AI_RESPONSE_INVALID:
      return 'AI returned an invalid response. Please try again.';
    default:
      return error.message || 'An unexpected error occurred.';
  }
}
