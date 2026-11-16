/**
 * Waits on you: every decision wanted from the person, with why it waits and what each answer
 * does, and one button per answer allowed. Below, what is going by itself and may still be
 * stopped or started at once.
 */

import { useState } from 'react';
import { useClient, useNow, useQuery } from '@cortex/sdk';
import { CHANGED, count, loadFailure, useAct, waiting, type Ask, type Going, type Inbox, type Repo, type Selection } from './manage';

const ACTION_LABEL: Record<string, string> = {
  approve: 'Approve',
  send_back: 'Send back',
  decline: 'Decline',
  rerun: 'Run again',
  take_over: 'Take over',
  first: 'First',
  second: 'Second',
};

function actionLabel(action: string): string {
  const known = ACTION_LABEL[action];
  if (known) return known;
  const words = action.replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Answers that go with a line from the person: what to change, or why not. */
const WITH_NOTE = new Set(['send_back', 'decline']);

function AskRow({ ask, repoName, now, open, changed }: { ask: Ask; repoName: string; now: number; open: (next: Selection) => void; changed: () => void }) {
  const client = useClient();
  const act = useAct();
  const [noting, setNoting] = useState<string | null>(null);
  const [note, setNote] = useState('');
  const decide = (action: string, text: string) => void act.run('decide', `"${actionLabel(action)}"`, async () => {
    await client.call('manage.decide', { id: ask.id, attempt: ask.attempt, action, note: text });
    setNoting(null);
    setNote('');
  }).then(changed);
  // Sending back without saying what to change gives the planner nothing to go on.
  const noteNeeded = noting === 'send_back';
  return (
    <li className="unread" data-ask={ask.id}>
      <span className="mk" />
      <div>
        <div className="top">
          {ask.request
            ? <button type="button" className="who" title="Open the request" onClick={() => open({ view: 'session', repo: ask.repo, request: ask.request, package: ask.package || null })}>{ask.title}</button>
            : <span className="who">{ask.title}</span>}
          <span className="acct work">{repoName}</span>
          <span className="when">{waiting(ask.since_ms, now)}</span>
        </div>
        <div className="subj">{ask.subject}</div>
        {ask.why && <div className="snip">{ask.why}</div>}
        {ask.if_yes && <div className="snip"><span className="ap faint">If yes</span>{ask.if_yes}</div>}
        {ask.if_no && <div className="snip"><span className="ap faint">If no</span>{ask.if_no}</div>}
        <div className="acts">
          {ask.actions.map((action) => (
            <button key={action} type="button" className="detent" aria-pressed={noting === action} disabled={act.busy !== null}
              onClick={() => (WITH_NOTE.has(action) ? setNoting(noting === action ? null : action) : decide(action, ''))}>
              {actionLabel(action)}
            </button>
          ))}
        </div>
        {noting && (
          <div className="cm-note-row">
            <label className="field">
              <input value={note} autoFocus placeholder={noteNeeded ? 'What should change' : 'Why, in one line'} aria-label="Your note" onChange={(event) => setNote(event.target.value)} />
            </label>
            <button type="button" className="detent send" aria-pressed={!noteNeeded || Boolean(note.trim())} disabled={act.busy !== null || (noteNeeded && !note.trim())} onClick={() => decide(noting, note.trim())}>
              {actionLabel(noting)}
            </button>
            <button type="button" className="detent" onClick={() => { setNoting(null); setNote(''); }}>Cancel</button>
          </div>
        )}
        {act.failed('decide') && <p className="inline-error cm-fail">{act.failed('decide')}</p>}
      </div>
    </li>
  );
}

function GoingRow({ going, repoName, changed }: { going: Going; repoName: string; changed: () => void }) {
  const client = useClient();
  const act = useAct();
  const edit = (key: string, doing: string, params: Record<string, unknown>) =>
    void act.run(key, doing, () => client.call('manage.package.edit', { id: going.package, ...params })).then(changed);
  return (
    <li data-package={going.package}>
      <span className="mk" />
      <div>
        <div className="top">
          <span className="who">{going.title}</span>
          <span className="acct work">{repoName}</span>
          <span className="when">{going.line}</span>
        </div>
        <div className="acts">
          <button type="button" className="detent" disabled={act.busy !== null} onClick={() => edit('stop', 'Stopping it', { stop: true })}>Stop it</button>
          {/* Only what still waits on the clock can be started ahead of it. */}
          {going.hold_until_ms > 0 && (
            <button type="button" className="detent" disabled={act.busy !== null} onClick={() => edit('start', 'Starting it now', { start_now: true })}>Start now</button>
          )}
        </div>
        {(act.failed('stop') ?? act.failed('start')) && <p className="inline-error cm-fail">{act.failed('stop') ?? act.failed('start')}</p>}
      </div>
    </li>
  );
}

export function InboxPage({ repos, open }: { repos: Repo[]; open: (next: Selection) => void }) {
  const inbox = useQuery<Inbox>('manage.inbox.list', (client) => client.call<Inbox>('manage.inbox.list', {}), { topics: [CHANGED] });
  const asks = inbox.data?.asks ?? [];
  const going = inbox.data?.going ?? [];
  // "waiting 2 h" moves on by itself.
  const now = useNow(60_000, asks.length > 0);
  const nameOf = (id: string) => repos.find((repo) => repo.id === id)?.name ?? id;
  const changed = () => void inbox.refresh();
  return (
    <>
      <div className="colhead">
        <span className="ap">Waits on you</span>
        {inbox.data && <span className="ap faint">{asks.length === 0 ? 'nothing' : count(asks.length, 'decision')}</span>}
      </div>
      <div className="cm-page">
        {inbox.error && !inbox.data && <p className="inline-error">{loadFailure('What waits on you', inbox)}</p>}
        {inbox.data && asks.length === 0 && <p className="cm-line">Nothing waits on you.</p>}
        {asks.length > 0 && (
          <ul className="arr cm-arr">
            {asks.map((ask) => <AskRow key={`${ask.id}:${ask.attempt}`} ask={ask} repoName={nameOf(ask.repo)} now={now} open={open} changed={changed} />)}
          </ul>
        )}
        {going.length > 0 && (
          <>
            <div className="colhead cm-subhead"><span className="ap faint">Going by themselves · nothing to do</span></div>
            <ul className="arr cm-arr">
              {going.map((item) => <GoingRow key={item.package} going={item} repoName={nameOf(item.repo)} changed={changed} />)}
            </ul>
          </>
        )}
      </div>
    </>
  );
}
