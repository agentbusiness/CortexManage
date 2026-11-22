/**
 * Timeline: what happened to every repository's requests over the last seven days, newest first,
 * grouped by day, each with why (`manage.timeline.read`). Pressing a line opens it in place: the
 * whole reason, who decided, what it points at, and a box to write what should happen now and
 * reopen it (`manage.request.reopen`). Earlier shows the week before.
 */

import { useState } from 'react';
import { useClient, useQuery } from '@cortex/sdk';
import { CHANGED, TIER_LABEL, loadFailure, useAct, type RequestView, type TimelineEntry } from './manage';

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

/** A decision that waited on a person (`manage.request.story`). */
interface Decision {
  kind: string;
  subject: string;
  why: string;
  since_ms: number;
  open: boolean;
  action: string;
  by: string;
  note: string;
  answered_ms: number;
}

interface Story {
  request: RequestView & { urgent_because?: string; merged_into?: string; follows?: string };
  because: string;
  events: TimelineEntry[];
  decisions: Decision[];
  merged: Array<{ id: string; title: string; from: string; ms: number }>;
}

/** How a decision was answered, in a person's words. */
const ANSWER: Record<string, string> = {
  approve: 'Approved',
  send_back: 'Sent back',
  decline: 'Declined',
  rerun: 'Tried again',
  take_over: 'Taken over',
  first: 'The first kept',
  second: 'The second kept',
  give_back: 'Planned again',
  superseded: 'No longer needed',
  landed: 'Settled when it landed',
  withdrawn: 'Withdrawn',
};

const cap = (text: string) => (text ? text[0]!.toUpperCase() + text.slice(1) : text);

function when(ms: number, now: number): string {
  if (!ms) return '';
  const day = dayName(ms, now);
  return day === 'Today' ? clock(ms) : `${day} ${clock(ms)}`;
}

/** The whole story of one request, under its line. */
function StoryView({ entry, now, onChat }: { entry: TimelineEntry; now: number; onChat: (request: string, repo: string) => void }) {
  const client = useClient();
  const act = useAct();
  const [text, setText] = useState('');
  const [done, setDone] = useState<string | null>(null);
  const story = useQuery<Story>(entry.request ? `manage.request.story:${entry.request}` : null, (c) => c.call<Story>('manage.request.story', { id: entry.request }), { topics: [CHANGED] });
  const reopen = () =>
    void act.run('reopen', 'Reopening it', async () => {
      const view = await client.call<{ id: string }>('manage.request.reopen', { id: entry.request, text: text.trim() });
      setText('');
      setDone(view?.id && view.id !== entry.request ? `Reopened as ${view.id}` : 'Reopened');
    });
  if (!entry.request) {
    return (
      <div className="cm-tl-story">
        <span className="ap">What happened</span>
        <p className="cm-tl-body">{entry.why}</p>
      </div>
    );
  }
  const s = story.data;
  const r = s?.request;
  const sorted = r && r.kind ? [cap(r.kind), TIER_LABEL[r.tier], r.surface.map(cap).join(' + ')].filter(Boolean).join(' · ') : '';
  const from = r?.from?.kind === 'app' ? r.from.name : r?.from?.kind === 'policy' ? cap(r.from.name) : 'You';
  return (
    <div className="cm-tl-story">
      {story.error && !s && <p className="inline-error cm-tl-wide">{loadFailure('The story of this request', story)}</p>}
      {r && (
        <>
          <span className="ap">Came in</span>
          <div>
            <p className="cm-tl-meta">{from} · {when(r.created_ms, now)} · {r.id}</p>
            <blockquote className="cm-tl-quote">{r.text}</blockquote>
            {(s.merged.length > 0) && <p className="cm-tl-meta">Also reported {s.merged.length} more {s.merged.length === 1 ? 'time' : 'times'}{s.merged.some((m) => m.from) ? ` by ${[...new Set(s.merged.map((m) => m.from).filter(Boolean))].join(', ')}` : ''}</p>}
          </div>

          {sorted && (
            <>
              <span className="ap">Sorted as</span>
              <div>
                <p className="cm-tl-body">{sorted}{r.urgent ? ' · Urgent' : ''}</p>
                {s.because && <p className="cm-tl-note">{s.because}</p>}
                {r.urgent && r.urgent_because && <p className="cm-tl-note">Urgent because: “{r.urgent_because}”</p>}
              </div>
            </>
          )}

          {r.plan && r.plan.points.length > 0 && (
            <>
              <span className="ap">Plan</span>
              <ol className="cm-tl-plan">
                {r.plan.points.map((point) => (
                  <li key={point.n} className={point.verdict === 'cut' ? 'cut' : undefined}>
                    <span className="t">{point.title}</span>
                    {point.verdict && point.verdict !== 'keep' && <span className="v">{point.verdict === 'cut' ? 'Left out' : cap(point.verdict)}{point.verdict_reason ? `: ${point.verdict_reason}` : ''}</span>}
                  </li>
                ))}
              </ol>
            </>
          )}

          {s.decisions.length > 0 && (
            <>
              <span className="ap">Waited on you</span>
              <ul className="cm-tl-list">
                {s.decisions.map((decision, index) => (
                  <li key={index}>
                    <span className="cm-tl-when">{when(decision.since_ms, now)}</span>
                    <span>
                      <span className="cm-tl-body">{decision.subject}</span>
                      {decision.why && <span className="cm-tl-note">{decision.why}</span>}
                      <span className={`cm-tl-answer${decision.open ? ' open' : ''}`}>
                        {decision.open ? 'Waiting for your answer' : `${ANSWER[decision.action] ?? cap(decision.action)}${decision.by === 'person' ? ' by you' : ''}${decision.answered_ms ? ` · ${when(decision.answered_ms, now)}` : ''}${decision.note ? ` · “${decision.note}”` : ''}`}
                      </span>
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}

          <span className="ap">What happened</span>
          <ul className="cm-tl-list">
            {s.events.map((event, index) => (
              <li key={index}>
                <span className="cm-tl-when">{when(event.ms, now)}</span>
                <span>
                  <span className={`cm-tl-ev${ENDED.has(event.event) ? ' end' : ''}`}>{EVENT[event.event] ?? event.event}</span>
                  <span className="cm-tl-body">{event.why}</span>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}

      <span />
      <div className="cm-tl-acts">
        {entry.reopen && (
          <textarea className="cm-text" rows={2} value={text} aria-label="What should happen now" placeholder="What should happen now" onChange={(event) => setText(event.target.value)} />
        )}
        <div className="cm-tl-btns">
          <button type="button" className="detent" onClick={() => onChat(entry.request, entry.repo)}>Open chat</button>
          {entry.reopen && <button type="button" className="detent send" disabled={!text.trim() || act.busy !== null} onClick={reopen}>Reopen</button>}
          {done && <span className="cm-tl-said" role="status">{done}</span>}
        </div>
        {act.failed('reopen') && <p className="inline-error">{act.failed('reopen')}</p>}
      </div>
    </div>
  );
}

function Entry({ entry, open, now, onToggle, onChat }: { entry: TimelineEntry; open: boolean; now: number; onToggle: () => void; onChat: (request: string, repo: string) => void }) {
  return (
    <li className={`cm-tl-row${open ? ' open' : ''}`} data-event={entry.event} data-request={entry.request}>
      <button type="button" className="cm-tl-line" aria-expanded={open} onClick={onToggle}>
        <span className="cm-tl-t">{clock(entry.ms)}</span>
        <span className="cm-tl-repo">{entry.repo_name}</span>
        <span className="cm-tl-title">{entry.title || entry.repo_name}</span>
        <span className={`cm-tl-ev${ENDED.has(entry.event) ? ' end' : ''}`}>{EVENT[entry.event] ?? entry.event}</span>
        <span className="cm-tl-why">{entry.why}</span>
      </button>
      {open && <StoryView entry={entry} now={now} onChat={onChat} />}
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
              return <Entry key={key} entry={entry} open={open === key} now={now} onToggle={() => setOpen(open === key ? null : key)} onChat={openRequest} />;
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
