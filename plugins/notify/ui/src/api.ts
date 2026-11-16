/**
 * The notify@1 contract as the settings panel uses it (protocol/src/contracts/notify.rs).
 */

export interface KindPref {
  kind: string;
  label: string;
  description: string;
  enabled: boolean;
  default: boolean;
}

export interface PrefsResult {
  kinds: KindPref[];
  only_when_away: boolean;
  long_job_ms: number;
  dedup_window_ms: number;
  /** settings | defaults */
  source: string;
  detail: string;
}

export interface NativeStatus {
  method: string;
  activation_capable: boolean;
  helper_path: string;
  authorization: string;
  osascript: boolean;
  detail: string;
}

export interface StatusResult {
  native: NativeStatus;
  deliverer?: { deliverer_id: string; kind: string; age_ms: number; ttl_ms: number } | null;
  focus?: { window_focused: boolean; visible: boolean; session_id?: string | null } | null;
  focus_age_ms?: number | null;
  prefs_source: string;
  prefs_detail: string;
  history: number;
}

export type PrefsChange =
  | { kind: string; enabled: boolean }
  | { only_when_away: boolean };

/** The native path, in words. */
export function nativeText(status: NativeStatus): string {
  if (status.method === 'helper') return 'On. macOS shows them; clicking one opens the exact place in CortexManage.';
  switch (status.authorization) {
    case 'denied':
      return 'Off in macOS. Turn on notifications for CortexManage in System Settings > Notifications; until then CortexManage only marks the session in the list.';
    case 'not_determined':
      return 'macOS is asking whether CortexManage may send notifications; until you answer CortexManage only marks the session in the list.';
    case 'absent':
      return `The CortexManage notification app is missing${status.detail ? ` (${status.detail})` : ''}; CortexManage only marks the session in the list.`;
    default:
      return `The CortexManage notification app reports ${status.authorization}${status.detail ? ` (${status.detail})` : ''}; CortexManage only marks the session in the list.`;
  }
}
