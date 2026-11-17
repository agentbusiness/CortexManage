/**
 * workbench.band@1 — the window's two controls, at the right of the brand band: Open pane or
 * Close pane (the right pane's one switch) and the settings window's gear (`settings.host@1`,
 * which this package also places over the window while it is open).
 */

import { definePlugin, useHost, useShared } from '@cortex/sdk';
import './controls.css';

function Controls() {
  const host = useHost();
  const [settings, setSettings] = useShared<string | null>('settings.open', null);
  const [, setOverlay] = useShared<boolean>('overlay.open', false);
  const [collapsed, setCollapsed] = useShared<boolean>('layout.paneCollapsed', false);

  const openSettings = () => {
    // '' opens the settings window on its first panel.
    setSettings('');
    setOverlay(true);
  };
  const closeSettings = () => {
    setSettings(null);
    setOverlay(false);
  };

  return (
    <div className="cx-band-controls" data-build={__CORTEX_BUILD__}>
      <button type="button" className="detent cx-pane-open" data-action={collapsed ? 'open-pane' : 'close-pane'} aria-pressed={!collapsed} onClick={() => setCollapsed(!collapsed)}
        title={collapsed ? 'Open the right pane again, at its last width and tab' : 'Close the right pane; the centre takes the width'}>{collapsed ? 'Open pane' : 'Close pane'}</button>
      <button type="button" className="detent cx-band-gear" onClick={() => (settings === null ? openSettings() : closeSettings())} data-action="settings" aria-pressed={settings !== null} title="Settings" aria-label="Settings">
        <Gear />
      </button>
      {settings !== null && <host.Slot name="settings.host@1" className="cx-settings-layer" props={{ onClose: closeSettings }} />}
    </div>
  );
}

function Gear() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3" aria-hidden="true">
      <circle cx="8" cy="8" r="2.2" />
      <path d="M6.9 1.5h2.2l.4 1.9 1.3.6 1.6-1.1 1.6 1.6-1.1 1.6.6 1.3 1.9.4v2.2l-1.9.4-.6 1.3 1.1 1.6-1.6 1.6-1.6-1.1-1.3.6-.4 1.9H6.9l-.4-1.9-1.3-.6-1.6 1.1-1.6-1.6 1.1-1.6-.6-1.3-1.9-.4V6.9l1.9-.4.6-1.3-1.1-1.6 1.6-1.6 1.6 1.1 1.3-.6z" strokeLinejoin="round" />
    </svg>
  );
}

export default definePlugin({ slot: 'workbench.band@1', component: Controls });
