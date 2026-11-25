/// PackStudio SemVer 2.0 base version and build-time short Git commit metadata.
export const APP_BASE_VERSION: string =
  import.meta.env.VITE_APP_VERSION || "0.3.1-beta";

export const APP_COMMIT_SHA: string =
  import.meta.env.VITE_APP_COMMIT_SHA || "dev";

export const APP_DISPLAY_VERSION: string = `${APP_BASE_VERSION}+${APP_COMMIT_SHA}`;
