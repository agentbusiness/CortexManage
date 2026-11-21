/**
 * workbench.band@1 — Needs you: shown only while a major change waits for a person's yes. Pressing
 * it opens the board, where the decision is at the top.
 */

import { definePlugin, setShared, useQuery } from '@cortex/sdk';
import { CHANGED, LAYOUT_BOARD, local, type Inbox } from './manage';
import './band.css';

function NeedsYou() {
  const inbox = useQuery<Inbox>('manage.inbox.list', (client) => client.call<Inbox>('manage.inbox.list', {}), { topics: [CHANGED] });
  const waiting = inbox.data?.asks?.filter((ask) => ask.kind !== 'clarify').length ?? 0;
  if (waiting === 0) return null;
  return (
    <span className="cm-band" data-build={__CORTEX_BUILD__} data-waiting={waiting}>
      <button type="button" className="detent" aria-pressed title="A major change waits for your yes" onClick={() => setShared(LAYOUT_BOARD, true)}>
        Needs you <span className="k">{waiting}</span>
      </button>
    </span>
  );
}

export default definePlugin({ slot: 'workbench.band@1', component: local(NeedsYou) });
