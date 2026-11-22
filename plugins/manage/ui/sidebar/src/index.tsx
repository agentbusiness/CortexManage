/**
 * workbench.sidebar@1 — the board, unfolded or folded.
 *
 * Unfolded (the home screen) it is the board over the whole work area. Folded it is the sidebar
 * beside the conversation: each repository with its requests, in the board's order (Incoming,
 * Waiting, Coding, Done), each saying where its work is. B and Fold fold it and show every repository's next release; Open chat folds it on a
 * conversation; Board unfolds it.
 */

import { useEffect, useState } from 'react';
import { definePlugin, setShared, useNow, useShared } from '@cortex/sdk';
import { COLUMNS, LAYOUT_BOARD, loadFailure, local, pipClass, sessionOf, useManageSelection, type BoardItem, type Repo, type Selection } from './manage';
import { BoardView, useBoard } from './Board';
import { stateLine } from './moves';
import { AddRepo } from './AddRepo';
import './manage.css';
import './sidebar.css';
import './board.css';

/** Fold shows every repository's next release. */
const SCHEDULE: Selection = { view: 'schedule', repo: null, request: null, package: null };

/** The board is the home screen: the window opens on it once per start. */
let homed = false;

const ORDER = Object.fromEntries(COLUMNS.map((column, index) => [column.key, index]));
const LABEL = Object.fromEntries(COLUMNS.map((column) => [column.key, column.label])) as Record<string, string>;

function RepoBlock({ repo, items, selection, open, now }: { repo: Repo; items: BoardItem[]; selection: Selection; open: (next: Selection) => void; now: number }) {
  const on = selection.repo === repo.id && selection.view !== 'inbox' && selection.view !== 'session';
  const sorted = [...items].sort((a, b) => (ORDER[a.column] ?? 0) - (ORDER[b.column] ?? 0) || Number(b.urgent) - Number(a.urgent));
  return (
    <section className={`cm-repo${repo.managed ? '' : ' off'}`} data-repo={repo.id}>
      <ul className="nav">
        <li className={on ? 'on' : undefined}>
          <button type="button" title={`${repo.name}: its charter, releases and builder`} onClick={() => open({ view: 'charter', repo: repo.id, request: null, package: null })}>
            <span className="nm">{repo.name}</span>
            {!repo.managed && <span className="sub">not managed yet</span>}
          </button>
        </li>
      </ul>
      <ul className="cm-sess">
        {sorted.map((item) => {
          const selected = selection.view === 'session' && selection.request === item.id;
          const attention = item.word === 'needs_you' || (item.urgent && item.column !== 'done' && item.column !== 'shipped');
          return (
            <li key={item.id} className={`${selected ? 'sel' : ''}${attention ? ' attn' : ''}`} data-request={item.id}>
              <button type="button" title={`${item.title} — ${stateLine(item, now)}`} aria-pressed={selected} onClick={() => open(sessionOf(item))}>
                <span className={pipClass(item.word)} />
                <span className="tt">{item.title || item.id}</span>
                <span className="st">{item.word === 'needs_you' ? 'Needs you' : item.urgent && attention ? 'Urgent' : LABEL[item.column]}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Folded({ now }: { now: number }) {
  const { repos, board, items } = useBoard();
  const [selection, setSelection] = useManageSelection();
  const [adding, setAdding] = useState(false);
  const list = repos.data ?? [];
  return (
    <div className="cm-side" data-build={__CORTEX_BUILD__}>
      <div className="cm-side-acts">
        <button type="button" className="detent" title="Open the board over the whole window (B)" onClick={() => setShared(LAYOUT_BOARD, true)}>Board</button>
        <button type="button" className="detent" aria-pressed={selection.view === 'schedule'} onClick={() => setSelection(SCHEDULE)}>Releases</button>
        <button type="button" className="detent" aria-pressed={selection.view === 'new'}
          onClick={() => setSelection({ view: 'new', repo: selection.repo ?? list.find((repo) => repo.managed)?.id ?? null, request: null, package: null })}>New request</button>
        <button type="button" className="detent" aria-pressed={adding} onClick={() => setAdding(!adding)}>Add repo</button>
      </div>
      {adding && <AddRepo onClose={() => setAdding(false)} onAdded={(next) => { setAdding(false); setSelection(next); void repos.refresh(); }} />}
      <div className="cm-side-scroll">
        {repos.error && !repos.data && <p className="inline-error">{loadFailure('The repositories', repos)}</p>}
        {board.error && !board.data && <p className="inline-error">{loadFailure('The requests', board)}</p>}
        {repos.data && list.length === 0 && <p className="cm-line flush">No repository yet. Add one with Add repo.</p>}
        {list.map((repo) => (
          <RepoBlock key={repo.id} repo={repo} items={items.filter((item) => item.repo === repo.id)} selection={selection} open={setSelection} now={now} />
        ))}
      </div>
    </div>
  );
}

function Sidebar() {
  const [unfolded, setUnfolded] = useShared<boolean>(LAYOUT_BOARD, false);
  const [, setSelection] = useManageSelection();
  // Times on the cards turn from a clock into a day at midnight.
  const now = useNow(60_000, true);
  useEffect(() => {
    if (!homed) {
      homed = true;
      setUnfolded(true);
    }
  }, [setUnfolded]);
  // B folds and unfolds the board, unless the person is typing.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'b' && event.key !== 'B') return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return;
      event.preventDefault();
      if (unfolded) setSelection(SCHEDULE);
      setUnfolded(!unfolded);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [unfolded, setUnfolded, setSelection]);
  return unfolded ? <BoardView now={now} /> : <Folded now={now} />;
}

export default definePlugin({ slot: 'workbench.sidebar@1', component: local(Sidebar) });
