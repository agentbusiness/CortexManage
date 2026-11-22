/**
 * The releases of every repository, one under another: how long until each leaves and the
 * requests going into it (done, coding, waiting). A repository with none of those is not shown.
 * Fold opens it; pressing a request opens its chat.
 */

import { useQuery } from '@cortex/sdk';
import { CHANGED, loadFailure, sessionOf, span, timeOf, type BoardItem, type Column, type Repo, type Selection } from './manage';

interface TrainView {
  next_ms: number;
  waits?: string[];
}

const GOING: Column[] = ['done', 'coding', 'waiting'];
const LABEL: Record<string, string> = { done: 'Ready', coding: 'Coding', waiting: 'Waiting' };

function RepoRelease({ repo, items, now, open }: { repo: Repo; items: BoardItem[]; now: number; open: (next: Selection) => void }) {
  const train = useQuery<TrainView>(`train.get:${repo.id}`, async (client) => (await client.call<TrainView>('train.get', { repo: repo.id })) ?? { next_ms: 0 }, { topics: ['train.changed', CHANGED] });
  const next = train.data?.next_ms ?? 0;
  const when = repo.delivery?.release_kind === 'none'
    ? 'No release'
    : next
      ? `${next > now ? `in ${span(next - now)}` : 'now'} · ${timeOf(next, now)}`
      : '';
  const waits = train.data?.waits ?? [];
  const sorted = [...items].sort((a, b) => GOING.indexOf(a.column) - GOING.indexOf(b.column) || Number(b.urgent) - Number(a.urgent));
  return (
    <section className="cm-sched-repo" data-repo={repo.id}>
      <div className="cm-sched-h">
        <span className="nm">{repo.name}</span>
        <span className="grow" />
        {waits.length > 0 && <span className="ap faint">Waits for {waits.join(', ')}</span>}
        <span className="ap">{when}</span>
      </div>
      <ul className="cm-sched-list">
        {sorted.map((item) => (
          <li key={item.id}>
            <button type="button" onClick={() => open(sessionOf(item))}>
              <span className="t">{item.title || item.id}</span>
              <span className="ap faint">{item.urgent ? 'Urgent · ' : ''}{LABEL[item.column]}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function SchedulePage({ repos, open, now }: { repos: Repo[]; open: (next: Selection) => void; now: number }) {
  const board = useQuery<{ items: BoardItem[] }>(
    'manage.board.items',
    async (client) => (await client.call<{ items: BoardItem[] }>('manage.board.items', {})) ?? { items: [] },
    { topics: [CHANGED, 'train.changed'] },
  );
  const going = (board.data?.items ?? []).filter((item) => GOING.includes(item.column));
  const shown = repos.filter((repo) => going.some((item) => item.repo === repo.id));
  return (
    <div className="cm-page pad cm-sched">
      {board.error && !board.data && <p className="inline-error">{loadFailure('The requests', board)}</p>}
      {board.data && shown.length === 0 && <p className="cm-line flush">Nothing is going into a release.</p>}
      {shown.map((repo) => (
        <RepoRelease key={repo.id} repo={repo} items={going.filter((item) => item.repo === repo.id)} now={now} open={open} />
      ))}
    </div>
  );
}
