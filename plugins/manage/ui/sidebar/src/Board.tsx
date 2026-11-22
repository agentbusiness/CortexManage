/**
 * The board: four columns (Incoming, Waiting, Coding, Done), each repository a row. A card is one
 * request. Pressing it shows its details beside the board; Open chat folds the board into the
 * sidebar and shows its conversation. Dragging does the obvious: into Coding starts it, back to
 * Waiting pauses it, onto another waiting card builds the two together, out of a together bracket
 * builds it on its own. A major change that waits for a yes is in the strip at the top.
 */

import { useEffect, useRef, useState, type DragEvent, type ReactElement } from 'react';
import { Menu, setShared, useClient, useQuery, useSlotState, type Client } from '@cortex/sdk';
import {
  CHANGED,
  COLUMNS,
  LAYOUT_BOARD,
  loadFailure,
  sessionOf,
  useAct,
  useManageSelection,
  type Ask,
  type BoardItem,
  type Column,
  type Repo,
  type Selection,
} from './manage';
import { dropOnCard, dropOnColumn, reportLine, stateLine, tagLine, type Move } from './moves';
import { AddRepo } from './AddRepo';
import { RepoSheet } from './RepoSheet';
import { FlowPage, useFlow, type Table } from './Flow';
import { TimelinePage } from './Timeline';
import { NextRelease, releaseLine, useTrain } from './NextRelease';
import './flow.css';
import './timeline.css';

/** The repository's next release in one line; pressing it opens the release under it. */
function ReleaseButton({ repo, items, now, open, onToggle }: { repo: Repo; items: BoardItem[]; now: number; open: boolean; onToggle: () => void }) {
  const train = useTrain(repo.id);
  return (
    <button type="button" className={`cm-rel-line${open ? ' on' : ''}`} aria-expanded={open} onClick={onToggle}>
      {releaseLine(repo, train.data, items, now)}
    </button>
  );
}

const listRepos = async (client: Client) => (await client.call<{ repos: Repo[] }>('manage.repo.list', {}))?.repos ?? [];

/** What the board and the folded sidebar both read. */
export function useBoard() {
  const repos = useQuery<Repo[]>('manage.repo.list', listRepos, { topics: [CHANGED] });
  const board = useQuery<{ items: BoardItem[]; asks: Ask[] }>(
    'manage.board.items',
    async (client) => (await client.call<{ items: BoardItem[]; asks: Ask[] }>('manage.board.items', {})) ?? { items: [], asks: [] },
    { topics: [CHANGED, 'train.changed'] },
  );
  return { repos, board, items: board.data?.items ?? [], asks: board.data?.asks ?? [] };
}

/** Open a request's details beside the board. */
function showDetails(setSelection: (next: Selection) => void, item: BoardItem) {
  setSelection(sessionOf(item));
  setShared('pane.tab', 'card');
  setShared('layout.paneCollapsed', false);
}

/** Fold the board and show something in the centre. */
function fold(setSelection: (next: Selection) => void, next: Selection) {
  setSelection(next);
  setShared(LAYOUT_BOARD, false);
}

function CardView({
  item,
  repo,
  now,
  selected,
  over,
  onOpen,
  onChat,
  onDragStart,
  onDragEnd,
  onDragOverCard,
  onDropCard,
}: {
  item: BoardItem;
  repo: string;
  now: number;
  selected: boolean;
  over: boolean;
  onOpen: () => void;
  onChat: () => void;
  onDragStart: (event: DragEvent) => void;
  onDragEnd: () => void;
  onDragOverCard: (event: DragEvent) => void;
  onDropCard: (event: DragEvent) => void;
}) {
  const tags = tagLine(item);
  const reports = reportLine(item);
  const wants = item.word === 'needs_you';
  // Done and shipped work is on the main branch: there is nothing to move it to.
  const fixed = item.column === 'done' || item.column === 'shipped';
  return (
    <div
      className={`cm-card${fixed ? ' fixed' : ''}${selected ? ' sel' : ''}${item.urgent ? ' urgent' : ''}${over ? ' over' : ''}${item.paused ? ' paused' : ''}`}
      data-request={item.id}
      draggable={!fixed}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      title={fixed ? `${item.title} — ${item.column === 'shipped' ? 'shipped in a release' : 'on the main branch, waiting for the next release'}. To change it, write a new request.` : `${item.title} — drag it to another column; press for details, twice for the chat`}
      onClick={onOpen}
      onDoubleClick={onChat}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onChat();
        else if (event.key === ' ') {
          event.preventDefault();
          onOpen();
        }
      }}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOverCard}
      onDrop={onDropCard}
    >
      {repo && <span className="repo">{repo}</span>}
      <span className="t">{item.title || item.id}</span>
      {tags && <span className="tags">{tags}</span>}
      {reports && <span className="from">{reports}</span>}
      <span className={`state${wants ? ' live' : ''}`}>
        {item.urgent && item.column !== 'done' && item.column !== 'shipped' && <span className="urgent-word">Urgent · </span>}
        {stateLine(item, now)}
      </span>
    </div>
  );
}

export function BoardView({ now }: { now: number }) {
  const { repos, board, items, asks } = useBoard();
  const client = useClient();
  const act = useAct();
  const [selection, setSelection] = useManageSelection();
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [said, setSaid] = useState<string | null>(null);
  const [overCard, setOverCard] = useState<string | null>(null);
  const [overCell, setOverCell] = useState<string | null>(null);
  const dragged = useRef<BoardItem | null>(null);
  // The repository the board shows; empty: all of them.
  const [only, setOnly] = useSlotState<string>('board.repo', '');
  // What the board area shows: the board, or the Error flow or Metrics flow page.
  const [page, setPage] = useSlotState<'board' | 'timeline' | Table>('board.page', 'board');
  // The repository whose next release is open under its name.
  const [releaseOpen, setReleaseOpen] = useState<string | null>(null);
  // The computer the flow pages show; empty: all of them.
  const [node, setNode] = useSlotState<string>('flow.node', '');
  // Metrics flow shows the model calls, or the methods the apps served.
  const [metrics, setMetrics] = useSlotState<'models' | 'calls'>('flow.metrics', 'models');
  const table: Table | null = page === 'board' || page === 'timeline' ? null : page === 'calls' ? metrics : page;
  const flow = useFlow(table, only, node);
  const togglePage = (next: Table | 'timeline') => setPage(page === next ? 'board' : next);
  // Where a card was dropped, until the board's next answer says where its work is.
  const [placed, setPlaced] = useState<Record<string, Column>>({});
  useEffect(() => setPlaced({}), [board.data]);
  const timer = useRef<number | null>(null);
  useEffect(() => () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
  }, []);

  const say = (text: string) => {
    setSaid(text);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setSaid(null), 3500);
  };
  const run = async (move: Move, item?: BoardItem, column?: Column) => {
    if (item && column) setPlaced((current) => ({ ...current, [item.id]: column }));
    if (await act.run('move', 'Moving it', () => client.call(move.method, move.params))) say(move.done);
    else if (item) setPlaced((current) => Object.fromEntries(Object.entries(current).filter(([id]) => id !== item.id)));
  };
  const decide = (ask: Ask, action: string) => void act.run(`ask:${ask.id}`, 'Answering', () => client.call('manage.decide', { id: ask.id, attempt: ask.attempt, action, note: '' }));

  const list = repos.data ?? [];
  const endDrag = () => {
    dragged.current = null;
    setOverCard(null);
    setOverCell(null);
  };
  /** A drop place: a column, of one repository's section when the board shows them all. */
  const cellHandlers = (column: Column, repo: string | null) => ({
    onDragOver: (event: DragEvent) => {
      const item = dragged.current;
      if (item && (repo === null || item.repo === repo) && dropOnColumn(item, column)) {
        event.preventDefault();
        setOverCell(`${repo ?? ''}|${column}`);
      }
    },
    onDragLeave: (event: DragEvent) => {
      if (!(event.currentTarget as Element).contains(event.relatedTarget as Node | null)) setOverCell(null);
    },
    onDrop: (event: DragEvent) => {
      const item = dragged.current;
      endDrag();
      const move = item && (repo === null || item.repo === repo) ? dropOnColumn(item, column) : null;
      if (move && item) {
        event.preventDefault();
        void run(move, item, column);
      }
    },
  });
  const card = (item: BoardItem, repoName: ((id: string) => string) | null) => (
    <CardView
      key={item.id}
      item={item}
      repo={repoName ? repoName(item.repo) : ''}
      now={now}
      selected={selection.view === 'session' && selection.request === item.id}
      over={overCard === item.id}
      onOpen={() => showDetails(setSelection, item)}
      onChat={() => fold(setSelection, sessionOf(item))}
      onDragStart={(event) => {
        dragged.current = item;
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('text/plain', item.id);
      }}
      onDragEnd={endDrag}
      onDragOverCard={(event) => {
        const from = dragged.current;
        if (from && dropOnCard(from, item)) {
          event.preventDefault();
          event.stopPropagation();
          setOverCard(item.id);
        }
      }}
      onDropCard={(event) => {
        const from = dragged.current;
        const move = from ? dropOnCard(from, item) : null;
        if (move) {
          event.preventDefault();
          event.stopPropagation();
          endDrag();
          void run(move);
        }
      }}
    />
  );
  /** A cell's cards in order, those built together in one bracket. */
  const cell = (cards: BoardItem[], repoName: ((id: string) => string) | null) => {
    const order = [...cards].sort((a, b) => Number(b.urgent) - Number(a.urgent) || (a.together || a.id).localeCompare(b.together || b.id) || a.id.localeCompare(b.id));
    const out: ReactElement[] = [];
    const seen = new Set<string>();
    for (const item of order) {
      if (!item.together) {
        out.push(card(item, repoName));
        continue;
      }
      if (seen.has(item.together)) continue;
      seen.add(item.together);
      out.push(
        <div key={`t:${item.together}`} className="cm-together">
          <span className="lab">Together</span>
          {order.filter((each) => each.together === item.together).map((each) => card(each, repoName))}
        </div>,
      );
    }
    return out;
  };

  const managed = list.filter((repo) => repo.managed);
  /** The repository's own pop-up (its releases, where it builds, …). */
  const settingsOf = (repo: string) => setEditing(repo);
  const editingRepo = list.find((repo) => repo.id === editing) ?? null;
  // One repository, or all of them: then each is a section of its own, one under the other.
  const chosen = list.find((repo) => repo.id === only) ?? null;
  // All repositories: a section per repository, one under another, each with its own lanes.
  const sections = !chosen && list.length > 0;
  const inView = chosen ? items.filter((item) => item.repo === chosen.id) : items;
  const columnOf = (item: BoardItem) => placed[item.id] ?? item.column;
  return (
    <div className="cm-board" data-build={__CORTEX_BUILD__}>
      <div className="cm-board-bar">
        {list.length > 0 && (
          <Menu
            label="Repository"
            value={chosen ? chosen.name : 'All repositories'}
            items={[
              { key: '', label: 'All repositories', selected: !chosen, onSelect: () => setOnly('') },
              ...list.map((repo) => ({ key: repo.id, label: repo.name, selected: chosen?.id === repo.id, onSelect: () => setOnly(repo.id) })),
            ]}
          />
        )}
        {table && (
          <Menu
            label="Computer"
            value={flow.computers.find((computer) => computer.node === node)?.name ?? (node || 'All computers')}
            items={[
              { key: '', label: 'All computers', selected: !node, onSelect: () => setNode('') },
              ...flow.computers.map((computer) => ({ key: computer.node, label: computer.name, selected: computer.node === node, onSelect: () => setNode(computer.node) })),
            ]}
          />
        )}
        {chosen && page === 'board' && <button type="button" className="detent" onClick={() => settingsOf(chosen.id)}>Settings</button>}
        {chosen && page === 'board' && chosen.managed && (
          <ReleaseButton repo={chosen} items={items.filter((item) => item.repo === chosen.id)} now={now} open={releaseOpen === chosen.id} onToggle={() => setReleaseOpen(releaseOpen === chosen.id ? null : chosen.id)} />
        )}
        {said && <span className="said" role="status">{said}</span>}
        <span className="grow" />
        {managed.length > 0 && (
          <button type="button" className="detent" onClick={() => fold(setSelection, { view: 'new', repo: (chosen ?? managed[0]!).id, request: null, package: null })}>New request</button>
        )}
        <button type="button" className="detent" onClick={() => setAdding(true)}>Add repo</button>
        <button type="button" className="detent" aria-pressed={page === 'timeline'} title={page === 'timeline' ? 'Back to the board' : 'What happened this week, and why'} onClick={() => togglePage('timeline')}>Timeline</button>
        <button type="button" className="detent" aria-pressed={page === 'errors'} title={page === 'errors' ? 'Back to the board' : 'What failed in the apps, by the minute'} onClick={() => togglePage('errors')}>Error flow</button>
        <button type="button" className="detent" aria-pressed={page === 'calls'} title={page === 'calls' ? 'Back to the board' : 'What the apps served, by the minute'} onClick={() => togglePage('calls')}>Metrics flow</button>
        <button type="button" className="detent" title="Fold the board into the sidebar (B)" onClick={() => fold(setSelection, { view: 'schedule', repo: null, request: null, package: null })}>Fold</button>
      </div>
      {editingRepo && <RepoSheet repo={editingRepo} onClose={() => { setEditing(null); void repos.refresh(); }} />}
      {adding && <AddRepo onClose={() => setAdding(false)} onAdded={(next) => { setAdding(false); fold(setSelection, next); }} />}
      {act.failed('move') && <p className="inline-error cm-board-note">{act.failed('move')}</p>}
      {repos.error && !repos.data && <p className="inline-error cm-board-note">{loadFailure('The repositories', repos)}</p>}
      {board.error && !board.data && <p className="inline-error cm-board-note">{loadFailure('The requests', board)}</p>}
      {table && <FlowPage table={table} flow={flow} openRequest={(request) => fold(setSelection, sessionOf(request))} onView={page === 'calls' ? setMetrics : undefined} />}
      {page === 'timeline' && <TimelinePage repo={only} openRequest={(request, repo) => fold(setSelection, { view: 'session', repo, request, package: null })} />}
      {page === 'board' && chosen && releaseOpen === chosen.id && (
        <div className="cm-board-note"><NextRelease repo={chosen} items={items.filter((item) => item.repo === chosen.id)} now={now} onOpen={(item) => showDetails(setSelection, item)} /></div>
      )}
      {page === 'board' && repos.data && list.length === 0 && <p className="cm-line">No repository yet. Add one with Add repo.</p>}
      {page === 'board' && asks.length > 0 && (
        <div className="cm-needs">
          {asks.map((ask) => {
            const item = items.find((each) => each.id === ask.request);
            return (
              <div key={ask.id} className="row">
                <span className="what">{ask.subject}</span>
                {ask.actions.includes('approve') && (
                  <button type="button" className="detent" disabled={act.busy !== null} onClick={() => decide(ask, 'approve')}>Approve</button>
                )}
                {item && <button type="button" className="detent" onClick={() => fold(setSelection, sessionOf(item))}>Open chat</button>}
                {act.failed(`ask:${ask.id}`) && <span className="inline-error">{act.failed(`ask:${ask.id}`)}</span>}
              </div>
            );
          })}
        </div>
      )}
      {page !== 'board' ? null : sections ? (
        <div className="cm-sw">
          <div className="cm-sw-head">
            {COLUMNS.map((column) => (
              <span key={column.key} className="nm">{column.label}<span className="n">{inView.filter((item) => columnOf(item) === column.key).length || ''}</span></span>
            ))}
          </div>
          <div className="cm-sw-body">
            {list.map((repo) => {
              const mine = inView.filter((item) => item.repo === repo.id);
              return (
                <section key={repo.id} className="cm-sw-repo" data-repo={repo.id}>
                  <div className="cm-sw-name">
                    <button type="button" title={`Show only ${repo.name}`} onClick={() => setOnly(repo.id)}>{repo.name}</button>
                    {!repo.managed && <span className="ap faint">not managed yet</span>}
                    <button type="button" className="detent" onClick={() => settingsOf(repo.id)}>Settings</button>
                    {repo.managed && <ReleaseButton repo={repo} items={mine} now={now} open={releaseOpen === repo.id} onToggle={() => setReleaseOpen(releaseOpen === repo.id ? null : repo.id)} />}
                  </div>
                  {releaseOpen === repo.id && <NextRelease repo={repo} items={mine} now={now} onOpen={(item) => showDetails(setSelection, item)} />}
                  <div className="cm-sw-cells">
                    {COLUMNS.map((column) => (
                      <div key={column.key} className={`cm-sw-cell${overCell === `${repo.id}|${column.key}` ? ' over' : ''}`} data-column={column.key} {...cellHandlers(column.key, repo.id)}>
                        {cell(mine.filter((item) => columnOf(item) === column.key), null)}
                      </div>
                    ))}
                  </div>
                </section>
              );
            })}
          </div>
        </div>
      ) : (
        <div className="cm-kanban">
          {COLUMNS.map((column) => {
            const mine = inView.filter((item) => columnOf(item) === column.key);
            return (
              <section key={column.key} className={`cm-col${overCell === `|${column.key}` ? ' over' : ''}`} data-column={column.key} {...cellHandlers(column.key, null)}>
                <header className="cm-col-head">
                  <span className="nm">{column.label}</span>
                  <span className="n">{mine.length || ''}</span>
                </header>
                <div className="cm-col-cards">{cell(mine, null)}</div>
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}
