/**
 * workbench.band@1 — Waits on you: how many decisions are wanted from the person (the button is
 * pressed while there is one), and beside it the count line of everything in motion. Pressing it
 * opens the inbox in the centre.
 */

import { definePlugin, useQuery } from '@cortex/sdk';
import { CHANGED, NO_SELECTION, loadFailure, local, useManageSelection, type Inbox } from './manage';
import './band.css';

function InboxBand() {
  const inbox = useQuery<Inbox>('manage.inbox.list', (client) => client.call<Inbox>('manage.inbox.list', {}), { topics: [CHANGED] });
  const [selection, setSelection] = useManageSelection();
  const waiting = inbox.data?.asks?.length ?? 0;
  return (
    <span className="cm-band" data-build={__CORTEX_BUILD__} data-waiting={waiting}>
      <button type="button" className="detent" aria-pressed={waiting > 0} data-open={selection.view === 'inbox' ? 'true' : 'false'}
        title={waiting > 0 ? 'Open what waits on your decision' : 'Nothing waits on you; open the inbox'}
        onClick={() => setSelection(NO_SELECTION)}>
        Waits on you <span className="k">{waiting}</span>
      </button>
      {inbox.data?.count_line && <span className="ap faint">{inbox.data.count_line}</span>}
      {inbox.error && !inbox.data && <span className="ap live cm-band-fail">{loadFailure('What waits on you', inbox)}</span>}
    </span>
  );
}

export default definePlugin({ slot: 'workbench.band@1', component: local(InboxBand) });
