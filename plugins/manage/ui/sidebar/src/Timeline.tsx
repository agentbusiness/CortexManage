/**
 * Timeline: what happened to every repository's requests over the last seven days, newest first,
 * grouped by day, each with why (`manage.timeline.read`). Pressing a line opens it in place: the
 * whole reason, who decided, what it points at, and a box to write what should happen now and
 * reopen it (`manage.request.reopen`). Earlier shows the week before.
 */

import { useState } from 'react';
import { useClient, useQuery } from '@cortex/sdk';
import { CHANGED, loadFailure, useAct, type TimelineEntry } from './manage';

const DAY_MS = 86_400_000;
const WEEK_MS = 7 * DAY_MS;

/** The one word an event is shown with. */
const EVENT: Record<string, string> = {
  arrived: 'New',
  merged: 'Joined',
  closed: 'Already done',
  declined: 'Declined',
  withdrawn: 'Withdrawn',
  reopened: 'Reopened',
  ready: 'Ready',
  landed: 'Landed',
  released: 'Shipped',
  release_failed: 'Release failed',
  delivery: 'Release method',
};

/** Events that ended something, drawn in the rubric so they are seen first. */
const ENDED = new Set(['merged', 'closed', 'declined', 'withdrawn', 'release_failed']);

const BY: Record<string, string> = { you: 'You', model: 'The model', rule: 'CortexManage', app: 'An app' };

const two = (n: number) => String(n).padStart(2, '0');
const clock = (ms: number) => {
  const date = new Date(ms);
  return `${two(date.getHours())}:${two(date.getMinutes())}`;
};

function dayName(ms: number, now: number): string {
  const start = (at: number) => new Date(new Date(at).toDateString()).getTime();
  const days = Math.round((start(now) - start(ms)) / DAY_MS);
  if (days === 0) return 'Today';
  if (days === 1) return 'Yesterday';
  return new Date(ms).toLocaleDateString('en', { weekday: 'long', day: 'numeric', month: 'short' });
}

interface Answer {
  entries: TimelineEntry[];
  since_ms: number;
  before_ms: number;
}

function Entry({ entry, open, onToggle, onChat }: { entry: TimelineEntry; open: boolean; onToggle: () => void; onChat: (request: string, repo: string) => void }) {
  const client = useClient();
  const act = useAct();
  const [text, setText] = useState('');
  const [done, setDone] = useState<string | null>(null);
  const word = EVENT[entry.event] ?? entry.event;
  const reopen = () =>
    void act.run('reopen', 'Reopening it', async () => {
      const view = await client.call<{ id: string }>('manage.request.reopen', { id: entry.request, text: text.trim() });
      setText('');
      setDone(view?.id && view.id !== entry.request ? `Reopened as ${view.id}.` : 'Reopened.');
    });
  return (
    <li className={`cm-tl-row${open ? ' open' : ''}`} data-event={entry.event} data-request={entry.request}>
      <button type="button" className="cm-tl-line" aria-expanded={open} onClick={onToggle}>
        <span className="cm-tl-t">{clock(entry.ms)}</span>
        <span className="cm-tl-repo">{entry.repo_name}</span>
        <span className="cm-tl-title">{entry.title || (entry.request ? entry.request : entry.repo_name)}</span>
        <span className={`cm-tl-ev${ENDED.has(entry.event) ? ' end' : ''}`}>{word}</span>
        <span className="cm-tl-why">{entry.why}</span>
      </button>
      {open && (
        <div className="cm-tl-more">
          <p className="cm-tl-full">{entry.why}</p>
          <div className="cm-tl-facts">
            {entry.request && <span><span className="ap">Request</span> {entry.request}</span>}
            {entry.by && <span><span className="ap">By</span> {BY[entry.by] ?? entry.by}</span>}
            {entry.link && <span><span className="ap">Points to</span> {entry.link}</span>}
            {entry.versions.length > 0 && <span><span className="ap">In</span> {entry.versions.join(', ')}</span>}
          </div>
          <div className="cm-tl-acts">
            {entry.request && <button type="button" className="detent" onClick={() => onChat(entry.request, entry.repo)}>Open chat</button>}
          </div>
          {entry.reopen && (
            <div className="cm-tl-reopen">
              <label className="field area">
                <textarea rows={2} value={text} aria-label="What should happen now" placeholder="What should happen now" onChange={(event) => setText(event.target.value)} />
              </label>
              <button type="button" className="detent send" disabled={!text.trim() || act.busy !== null} onClick={reopen}>Reopen</button>
            </div>
          )}
          {done && <p className="cm-tl-said" role="status">{done}</p>}
          {act.failed('reopen') && <p className="inline-error">{act.failed('reopen')}</p>}
        </div>
      )}
    </li>
  );
}

export function TimelinePage({ repo, openRequest }: { repo: string; openRequest: (request: string, repo: string) => void }) {
  // How many weeks back the page reaches (Earlier adds one).
  const [weeks, setWeeks] = useState(1);
  const [open, setOpen] = useState<string | null>(null);
  const timeline = useQuery<Answer>(
    `manage.timeline.read:${weeks}`,
    async (client) => {
      const now = Date.now();
      return (await client.call<Answer>('manage.timeline.read', { since_ms: now - weeks * WEEK_MS, before_ms: now })) ?? { entries: [], since_ms: 0, before_ms: 0 };
    },
    { topics: [CHANGED, 'train.changed'] },
  );
  const now = Date.now();
  const entries = (timeline.data?.entries ?? []).filter((entry) => !repo || entry.repo === repo);
  const days: Array<[string, TimelineEntry[]]> = [];
  for (const entry of entries) {
    const name = dayName(entry.ms, now);
    const last = days[days.length - 1];
    if (last && last[0] === name) last[1].push(entry);
    else days.push([name, [entry]]);
  }
  const keyOf = (entry: TimelineEntry, index: number) => `${entry.repo}|${entry.request}|${entry.event}|${entry.ms}|${index}`;
  return (
    <div className="cm-tl" data-weeks={weeks}>
      {timeline.error && !timeline.data && <p className="inline-error">{loadFailure('The timeline', timeline)}</p>}
      {timeline.data && entries.length === 0 && <p className="cm-tl-empty">Nothing happened in this time.</p>}
      {days.map(([name, list]) => (
        <section key={name} className="cm-tl-day">
          <h3 className="ap">{name}</h3>
          <ul>
            {list.map((entry, index) => {
              const key = keyOf(entry, index);
              return <Entry key={key} entry={entry} open={open === key} onToggle={() => setOpen(open === key ? null : key)} onChat={openRequest} />;
            })}
          </ul>
        </section>
      ))}
      {timeline.data && (
        <div className="cm-tl-foot">
          <button type="button" className="detent" onClick={() => setWeeks(weeks + 1)}>Earlier</button>
        </div>
      )}
    </div>
  );
}
