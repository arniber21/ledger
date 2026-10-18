/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * SSE URL for the verdict stream (#9). Unset means the dashboard stays empty.
   */
  readonly VITE_SENTINEL_SSE?: string;
  /**
   * POST URL for spec authoring SSE (#10, #16). Unset uses `/api/specs/author`.
   * An empty value means Specs reports that authoring is not connected and
   * does not invent a draft.
   */
  readonly VITE_SPEC_AUTHOR_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
