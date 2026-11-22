/**
 * The board's words and moves: the one line each card says, and what dragging a card means.
 * Everything is read from facts the manager counted; nothing here guesses.
 */

import { TIER_LABEL, timeOf, type BoardItem, type Column } from './manage';

const cap = (text: string) => (text ? text[0]!.toUpperCase() + text.slice(1) : text);

/** `Fix · Minor · Frontend`, or nothing before the request is sorted. */
export function tagLine(item: BoardItem): string {
  if (!item.kind) return '';
  return [cap(item.kind), TIER_LABEL[item.tier], item.surface.map(cap).join(' + ')].filter(Boolean).join(' · ');
}

/** Not planned yet (it is where the person put it, and goes on from there once planned). */
const early = (item: BoardItem) => ['arrived', 'clarifying', 'assessed', 'planned', 'validated'].includes(item.stage);

/** The one line a card says about where its work is. */
export function stateLine(item: BoardItem, now: number): string {
  switch (item.column) {
    case 'incoming':
      if (item.stage === 'arrived' || item.stage === 'clarifying') return 'Reading it';
      if (item.stage === 'assessed') return 'Planning';
      if (item.stage === 'planned') return 'Checking the plan';
      return 'Ready';
    case 'waiting':
      if (early(item)) return 'Planning, then waits for you';
      if (item.word === 'needs_you') return 'Needs your yes';
      if (item.paused) return 'Paused';
      if (item.starts_ms) return item.starts_ms <= now ? 'Starts next' : `Starts ${timeOf(item.starts_ms, now)}`;
      return 'Waiting';
    case 'coding':
      if (early(item)) return 'Planning, then starts';
      if (item.queued === 'after') return 'Starts after the work being coded now';
      if (item.queued === 'builder') return 'Waiting for a CortexCode to take it';
      if (item.queued === 'next') return 'Starting';
      if (item.word === 'needs_you') return 'Needs your yes';
      if (item.word === 'rework') return 'Fixing what a check found';
      if (item.checking) return 'Checking that it works';
      if (item.steps) return `Step ${Math.max(1, item.step)} of ${item.steps}`;
      return 'Coding';
    case 'shipped':
      return item.released_ms ? `Released ${timeOf(item.released_ms, now)}` : 'Released';
    case 'done':
      if (item.release_failed_ms) return 'Release failed · being repaired';
      if (item.stage === 'ready') return item.release_ms ? `Ready · ships ${timeOf(item.release_ms, now)}` : 'Ready · ships with the next release';
      if (item.release_ms) return `Releases ${timeOf(item.release_ms, now)}`;
      return 'Waiting for the next release';
  }
}

/** Who reported it, and how many times, when it came from apps. */
export function reportLine(item: BoardItem): string {
  const from = item.reporters.join(', ');
  if (item.reports > 0) return `×${item.reports + 1} reports${from ? ` · ${from}` : ''}`;
  return from ? `From ${from}` : '';
}

/** What a drop means: a call and the line the board then says. */
export interface Move {
  method: string;
  params: Record<string, unknown>;
  done: string;
}

/** A card dropped on a column. Every column takes a card from any other column. */
export function dropOnColumn(item: BoardItem, column: Column): Move | null {
  const request = (params: Record<string, unknown>, done: string): Move => ({ method: 'manage.request.edit', params: { id: item.id, ...params }, done });
  const pack = (params: Record<string, unknown>, done: string): Move => ({ method: 'manage.package.edit', params: { id: item.package, ...params }, done });
  if (column === item.column) {
    // Out of a Together bracket, within Waiting: it is built on its own.
    if (column === 'waiting' && item.together) return pack({ unbundle: item.id }, 'It is built on its own now.');
    return null;
  }
  switch (column) {
    case 'incoming':
      return request({ replan: true }, 'It is planned again.');
    case 'waiting':
      return request({ start: false }, 'It waits until you move it to Coding.');
    case 'coding':
      return request({ start: true }, item.column === 'incoming' ? 'It starts as soon as it is planned.' : 'Started.');
    case 'done':
      return request({ withdraw: true }, 'Taken off the board.');
    case 'shipped':
      // Only a release ships work; a card cannot be put there by hand.
      return null;
  }
}

/** A card dropped on another waiting card: the two are built together. */
export function dropOnCard(item: BoardItem, target: BoardItem): Move | null {
  if (item.id === target.id || item.repo !== target.repo) return null;
  if (item.column !== 'waiting' || target.column !== 'waiting' || !item.package || !target.package || item.package === target.package) return null;
  return { method: 'manage.package.edit', params: { id: item.package, join: target.id }, done: 'They are built together now.' };
}
