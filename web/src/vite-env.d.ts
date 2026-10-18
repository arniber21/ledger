/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * SSE URL for the verdict stream (#9). Unset means the dashboard stays empty.
   */
  readonly VITE_SENTINEL_SSE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
