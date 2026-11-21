/**
 * workbench.band@1 — Pause: one switch for everything. While it is on, nothing new starts; what
 * already runs goes on.
 */

import { definePlugin, useClient, useQuery } from '@cortex/sdk';
import { CHANGED, loadFailure, local, useAct } from './manage';
import './band.css';

function HoldBand() {
  const client = useClient();
  const hold = useQuery<{ on: boolean }>('manage.hold.get', (c) => c.call<{ on: boolean }>('manage.hold.get', {}), { topics: [CHANGED] });
  const act = useAct();
  const on = hold.data?.on === true;
  const flip = () => void act.run('hold', on ? 'Going on' : 'Pausing', async () => {
    await client.call('manage.hold.edit', { on: !on });
    await hold.refresh();
  });
  // While the service is not running the item beside this one already says so.
  const loadFailed = hold.error && !hold.data && hold.errorCode !== 'unavailable';
  return (
    <span className="cm-band" data-build={__CORTEX_BUILD__} data-hold={on ? 'on' : 'off'}>
      <button type="button" className="detent" aria-pressed={on} disabled={!hold.data || act.busy !== null}
        title={on ? 'Paused: nothing new starts. Press to go on.' : 'Pause: nothing new starts until you press it again.'}
        onClick={flip}>
        {on ? 'Paused' : 'Pause'}
      </button>
      {act.failed('hold') && <span className="ap live cm-band-fail">{act.failed('hold')}</span>}
      {loadFailed && <span className="ap live cm-band-fail">{loadFailure('Pause', hold)}</span>}
    </span>
  );
}

export default definePlugin({ slot: 'workbench.band@1', component: local(HoldBand) });
