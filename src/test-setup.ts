import '@testing-library/jest-dom';

// jsdom does not implement ResizeObserver — provide a minimal mock.
class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;
