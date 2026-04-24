/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_AUTH_BASE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
