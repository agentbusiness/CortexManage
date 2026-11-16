/**
 * manage.session@1 — one request's session: the column head (title, tier, kind, where it
 * reaches, urgent), the thread drawn from the session's journal with one block per kind of entry
 * the manage plugin writes, and the box to write in at the foot.
 *
 * Which request is open comes from the shared selection (`manage.selection`). Everything shown
 * is the manage plugin's (`manage.request.get`, the journal); everything pressed is one of its
 * methods, or this flow's `turn`.
 */

import { useCallback, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { definePlugin, errorMessage, kinds, useClient, useJournal, useQuery, useShared, type JournalEntry, type JournalPatch } from '@cortex/sdk';
import { Assessment, AskBlock, Clarify, GateBlock, Message, Note, PackageBlock, PlanBlock, StageLine, Trouble, WatchBlock, type Acts } from './blocks';
import { BEFORE_BUILDING, NO_SELECTION, wordLabel, type Ask, type RequestView, type Selection } from './types';
import './session.css';

/** This view's flow: what a person writes goes to its category. */
const FLOW = 'chatflow-manage';

interface FlowState { running: boolean; stage?: string | null }

const capital = (text: string): string => (text ? text[0]!.toUpperCase() + text.slice(1) : text);

/** The last entry of a kind, and the asks no later entry answers: what is still open in the thread. */
function openParts(entries: readonly JournalEntry[]) {
  let clarify = -1;
  let pkg = -1;
  const answered = new Set<string>();
  for (const entry of entries) {
    if (entry.kind === 'clarify') clarify = entry.seq;
    else if (entry.kind === 'package') pkg = entry.seq;
    else if (entry.kind === 'ask' && typeof entry.data.ask_id === 'string') answered.add(entry.data.ask_id);
  }
  return { clarify, pkg, answered };
}

function Session() {
  const client = useClient();
  const [selection] = useShared<Selection>('manage.selection', NO_SELECTION);
  const id = selection.view === 'session' ? selection.request : null;
  const loaded = useQuery<RequestView>(
    id ? `manage.request.get:${id}` : null,
    (c) => c.call<RequestView>('manage.request.get', { id }),
    { topics: ['manage.changed'] },
  );
  const request = loaded.data && loaded.data.id === id ? loaded.data : null;
  const sessionId = request?.session_id || null;
  const journal = useJournal(sessionId);
  const entries = journal.entries;
  const open = useMemo(() => openParts(entries), [entries]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');

  // Whether a message is on its way to the manager, or one of its questions runs for this session.
  const flow = useQuery<FlowState>(
    sessionId ? `${FLOW}.state:${sessionId}` : null,
    (c) => c.call<FlowState>(`${FLOW}.state`, { session_id: sessionId }),
    {
      topics: sessionId ? [`journal.${sessionId}`, 'manage.changed'] : [],
      when: (topic, data) => topic === 'manage.changed' || (data as JournalPatch | null)?.op === 'create',
    },
  );
  const working = flow.data?.running === true;

  const refresh = loaded.refresh;
  /** One contract call for a pressed control; the thread and the head follow from `manage.changed`. */
  const run = useCallback(async (method: string, params: unknown): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await client.call(method, params);
      await refresh();
      return true;
    } catch (failure) {
      setError(errorMessage(failure));
      return false;
    } finally {
      setBusy(false);
    }
  }, [client, refresh]);

  // The thread stays at its end while it grows, unless the person scrolled up to read.
  const thread = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const node = thread.current;
    if (node && pinned.current) node.scrollTop = node.scrollHeight;
  }, [entries, sessionId]);

  if (!id) return <main className="cm-session"><p className="empty-line">Choose a request in the sidebar to see its session.</p></main>;
  if (!request) {
    return (
      <main className="cm-session">
        <p className="empty-line">{loaded.error ? `This request could not be opened: ${loaded.error}` : 'Opening the request…'}</p>
      </main>
    );
  }

  const ended = Boolean(request.ended_as);
  const beforeBuilding = !ended && BEFORE_BUILDING.includes(request.stage);
  const acts: Acts = {
    request,
    busy,
    answer: (answers) => void run('manage.request.edit', { id: request.id, answers }),
    packageEdit: (pkg, edit) => void run('manage.package.edit', { id: pkg, ...edit }),
    decide: (ask: Ask, action) => void run('manage.decide', { id: ask.id, attempt: ask.attempt, action }),
  };

  const send = async () => {
    const text = draft.trim();
    if (!text || busy || !sessionId) return;
    pinned.current = true;
    if (await run(`${FLOW}.turn`, { session_id: sessionId, text })) setDraft('');
  };
  const onKey = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends; Shift+Enter is a new line. Not while a word is still being composed (IME).
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void send();
    }
  };

  const block = (entry: JournalEntry) => {
    switch (entry.kind) {
      case kinds.USER_MESSAGE: return <Message key={entry.seq} entry={entry} who="You" />;
      case kinds.ASSISTANT_MESSAGE: return <Message key={entry.seq} entry={entry} who="Manager" tone="live" />;
      case 'digest': return <Message key={entry.seq} entry={entry} who="Manager · digest" tone="live" />;
      case 'assessment': return <Assessment key={entry.seq} entry={entry} />;
      case 'clarify': return <Clarify key={entry.seq} entry={entry} acts={acts} open={!ended && request.stage === 'clarifying' && entry.seq === open.clarify} />;
      case 'manage_plan': return <PlanBlock key={entry.seq} entry={entry} />;
      case 'package': return <PackageBlock key={entry.seq} entry={entry} acts={acts} open={!ended && request.stage === 'held' && entry.seq === open.pkg} />;
      case 'watch': return <WatchBlock key={entry.seq} entry={entry} />;
      case 'gate': return <GateBlock key={entry.seq} entry={entry} />;
      case 'stage': return <StageLine key={entry.seq} entry={entry} />;
      case 'ask': {
        const ask = entry.data.ask as { id?: string } | undefined;
        return <AskBlock key={entry.seq} entry={entry} acts={acts} open={!ended && typeof ask?.id === 'string' && !open.answered.has(ask.id)} />;
      }
      case kinds.ERROR: return <Trouble key={entry.seq} entry={entry} />;
      case kinds.NOTE: return <Note key={entry.seq} entry={entry} />;
      default: return null;
    }
  };

  return (
    <main className="cm-session">
      <div className="colhead">
        <span className="ap title" title={request.title}>{request.title || request.id}</span>
        <span className="ap faint">{request.id}</span>
        <span className="grow" />
        {request.urgent && <span className="tag major">Urgent</span>}
        {request.kind && request.tier && <span className={`tag ${request.tier}`}>{request.tier}</span>}
        {request.kind && <span className="tag">{request.kind}</span>}
        {(request.surface ?? []).map((part) => <span className="tag" key={part}>{part}</span>)}
        {ended
          ? <span className="ap faint">{capital(request.ended_as ?? '')}</span>
          : request.word && <span className="ap origin">{capital(wordLabel(request.word))}</span>}
      </div>

      <div
        className="thread"
        ref={thread}
        onScroll={(event) => {
          const node = event.currentTarget;
          pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 40;
        }}
      >
        {journal.status === 'error' && <p className="empty-line">The session's journal could not be read: {journal.error}</p>}
        {entries.map(block)}
        {ended && request.ended_reason && <p className="aside sub">{capital(request.ended_as ?? '')}: {request.ended_reason}</p>}
      </div>

      <div className="composer cm-composer">
        <div className="composer-h">
          {working && (
            <>
              <span className="pip run" aria-hidden />
              <span className="ap origin">{flow.data?.stage === 'say' ? 'Passing it on' : `Working · ${flow.data?.stage ?? ''}`}</span>
              <button type="button" className="detent" onClick={() => void run(`${FLOW}.cancel`, { session_id: sessionId })}>Stop</button>
            </>
          )}
          {error && <span className="ap live" role="alert">{error}</span>}
          <span className="grow" />
          {beforeBuilding && (
            <>
              <span className="ap faint" id="cm-urgent">Urgent</span>
              <button
                type="button"
                className="sw"
                aria-labelledby="cm-urgent"
                aria-pressed={request.urgent === true}
                disabled={busy}
                onClick={() => void run('manage.request.edit', { id: request.id, urgent: !request.urgent })}
              />
            </>
          )}
        </div>
        <div className="write-row">
          <textarea
            className="write"
            rows={2}
            value={draft}
            disabled={ended}
            placeholder={ended ? 'This request has ended.' : beforeBuilding ? 'Add to the request, or say what to change in the plan…' : 'Tell the builder something…'}
            aria-label="Write to this request"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKey}
          />
          <button type="button" className="detent send" disabled={ended || busy || draft.trim() === ''} aria-pressed={!ended && !busy && draft.trim() !== ''} onClick={() => void send()}>
            {beforeBuilding || ended ? 'Send' : 'Steer'}
          </button>
        </div>
      </div>
    </main>
  );
}

export default definePlugin({ slot: 'manage.session@1', component: Session });
