/**
 * A repository's next release: when it leaves, what is ready for it, what is not ready yet, and
 * what it waits for. On the board each repository says it in one line; pressing the line opens
 * the list under it. "Wait for it" makes the release wait until that request is ready
 * (`manage.request.edit {must_ship}`): the only thing that makes a release wait.
 */

import { useClient, useQuery } from '@cortex/sdk';
import { timeOf, useAct, type BoardItem, type Repo } from './manage';

/** `train.get`'s answer (the fields read here). */
interface TrainView {
  next_ms: number;
  next: Array<{ part: string; from: string; to: string; carries: string[] }>;
  soaking: Array<{ package: string; state: string; requests: [string, string, string][] }>;
  waits?: string[];
}

export function useTrain(repo: string) {
  return useQuery<TrainView>(`train.get:${repo}`, async (client) => (await client.call<TrainView>('train.get', { repo })) ?? { next_ms: 0, next: [], soaking: [], waits: [] }, { topics: ['train.changed', 'manage.changed'] });
}

/** The repository's one line: `Next release 15:00 · 3 ready · 1 coming`. */
export function releaseLine(repo: Repo, train: TrainView | undefined, items: BoardItem[], now: number): string {
  const kind = repo.delivery?.release_kind;
  if (kind === 'none') return 'No release';
  const ready = items.filter((item) => item.column === 'done').length;
  const coming = items.filter((item) => item.column === 'coding' || item.column === 'waiting').length;
  const when = train?.next_ms ? `Next release ${timeOf(train.next_ms, now)}` : 'Next release';
  const waits = train?.waits?.length ? ' · waits' : '';
  return [when, ready ? `${ready} ready` : '', coming ? `${coming} coming` : ''].filter(Boolean).join(' · ') + waits;
}

export function NextRelease({ repo, items, now, onOpen }: { repo: Repo; items: BoardItem[]; now: number; onOpen: (item: BoardItem) => void }) {
  const client = useClient();
  const act = useAct();
  const train = useTrain(repo.id);
  const ready = items.filter((item) => item.column === 'done');
  const coming = items.filter((item) => item.column === 'coding' || item.column === 'waiting' || item.column === 'incoming');
  const parts = train.data?.next ?? [];
  const waits = train.data?.waits ?? [];
  const pin = (item: BoardItem) =>
    void act.run(`pin:${item.id}`, item.must_ship ? 'Not waiting for it' : 'Waiting for it', () => client.call('manage.request.edit', { id: item.id, must_ship: !item.must_ship }));
  return (
    <div className="cm-next" data-repo={repo.id}>
      <div className="cm-next-h">
        <span className="ap">{train.data?.next_ms ? `Leaves ${timeOf(train.data.next_ms, now)}` : 'Next release'}</span>
        {waits.length > 0 && <span className="cm-next-wait">Waits for {waits.join(', ')}</span>}
      </div>
      <div className="cm-next-cols">
        <section>
          <span className="ap faint">Ready</span>
          {ready.length === 0 && <span className="cm-next-none">Nothing yet</span>}
          {ready.map((item) => (
            <button key={item.id} type="button" className="cm-next-item" onClick={() => onOpen(item)}>
              <span className="t">{item.title || item.id}</span>
              <span className="s">{item.stage === 'ready' ? 'on its branch' : 'on main'}</span>
            </button>
          ))}
        </section>
        <section>
          <span className="ap faint">Not ready yet</span>
          {coming.length === 0 && <span className="cm-next-none">Nothing</span>}
          {coming.map((item) => (
            <div key={item.id} className={`cm-next-item${item.must_ship ? ' pinned' : ''}`}>
              <button type="button" className="t" onClick={() => onOpen(item)}>{item.title || item.id}</button>
              <button type="button" className="detent" aria-pressed={!!item.must_ship} disabled={act.busy !== null} onClick={() => pin(item)}>
                {item.must_ship ? 'Waiting for it' : 'Wait for it'}
              </button>
            </div>
          ))}
        </section>
        {parts.length > 0 && (
          <section>
            <span className="ap faint">Will ship</span>
            {parts.map((part) => (
              <span key={part.part} className="cm-next-part">{part.part}{part.from || part.to ? ` ${part.from}${part.from && part.to ? ' → ' : ''}${part.to}` : ''}</span>
            ))}
          </section>
        )}
      </div>
      {coming.map((item) => act.failed(`pin:${item.id}`)).filter(Boolean).map((text) => <p key={text} className="inline-error">{text}</p>)}
    </div>
  );
}
