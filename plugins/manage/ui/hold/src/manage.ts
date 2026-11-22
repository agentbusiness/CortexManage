/**
 * What every package of the manage plugin's window shares: the shapes `manage@1` answers
 * (protocol/src/contracts/manage.rs), the one selection, the words and times a person reads, and
 * how a call is run so that its failure is said next to the control that made it.
 *
 * Each UI package builds on its own, so this file is the same in every package of this plugin
 * (as the scheduler's `types.ts` is). Change it in one and copy it to the others.
 */

import { createElement, useCallback, useState } from 'react';
import type { ComponentType, ReactElement } from 'react';
import { ComputerScope, RpcError, errorMessage, useQuery, useShared, type QueryResult } from '@cortex/sdk';

// ─── the contract's shapes ──────────────────────────────────────────────────

export type Tier = 'safe' | 'minor' | 'major';
export type Word = 'on_hold' | 'needs_you' | 'stuck' | 'rework' | 'working' | 'watching' | 'held' | 'queued' | 'done' | 'idle';

export interface Source {
  kind: string;
  url: string;
  main: string;
  /** `none` | `github-pr`. */
  changes: string;
  /** Who the jobs' commits are made as; both empty: git's own identity on the builder. */
  commit_name?: string;
  commit_email?: string;
  github?: string;
  /** The GitHub connection it is reached with; empty: the sign-in of Settings > GitHub. */
  connection?: string;
}

/** Where one value of a delivery profile was read. */
export interface DeliveryFact {
  field: string;
  path: string;
  line: number;
  quote: string;
}

/** How a repository ships (`Delivery`). */
export interface Delivery {
  /** empty | learning | learned | failed */
  state: string;
  error: string;
  learned_ms: number;
  reach_main: string;
  main_is: string;
  release_kind: string;
  release_steps: string[];
  plan_step: string;
  tag: string;
  version_kind: string;
  version_file: string;
  version_key: string;
  version_now: string;
  checks: string[];
  ships: string;
  ci: string;
  take_back: string;
  secrets: string[];
  secret_files: string[];
  secrets_from: string;
  facts: DeliveryFact[];
  set_by_person: string[];
}

/** One thing that happened to a request (`TimelineEntry`). */
export interface TimelineEntry {
  repo: string;
  repo_name: string;
  event: string;
  request: string;
  title: string;
  why: string;
  by: string;
  link: string;
  versions: string[];
  ms: number;
  reopen: boolean;
}

/** `[minutes, tokens, lines]`. */
export type Budget = [number, number, number];

export interface Policy {
  hold_safe_min: number;
  hold_minor_min: number;
  hold_max_min: number;
  soak_safe_min: number;
  soak_minor_min: number;
  soak_major_min: number;
  rework_safe: number;
  rework_minor: number;
  rework_major: number;
  budget_safe: Budget;
  budget_minor: Budget;
  budget_major: Budget;
  major_paths: string[];
  minor_paths: string[];
  /** `HH:MM`. */
  train_at: string;
  /** `3h` | `week` | empty: daily. */
  train_every?: string;
  /** For a weekly release: 0 Monday … 6 Sunday. */
  train_day?: number;
}

export interface Repo {
  id: string;
  name: string;
  prefix: string;
  software: string;
  source: Source;
  /** The runner's mesh node; empty: this computer. */
  runner: string;
  runner_name: string;
  policy: Policy;
  managed: boolean;
  created_ms: number;
  delivery?: Delivery;
  /** The person's rules for its commits and releases. */
  rules?: string;
}

export interface Walk {
  id: string;
  do: string;
  see: string;
  /** computer | command | mesh | journal. */
  method: string;
  evidence_kind: string;
  command: string;
  expect: string;
}

export interface Point {
  n: number;
  title: string;
  brief: string;
  touches: string[];
  walks: Walk[];
  verdict: string;
  verdict_reason: string;
  request: string;
}

export interface Plan {
  version: number;
  goal: string;
  points: Point[];
}

/** A request with what is computed about it (`RequestView`, flattened). */
export interface RequestView {
  id: string;
  repo: string;
  session_id: string;
  title: string;
  text: string;
  kind: string;
  tier: Tier;
  surface: string[];
  urgent: boolean;
  stage: string;
  ended_as: string;
  package: string;
  parts: string[];
  created_ms: number;
  updated_ms: number;
  word: Word;
  plan?: Plan;
  /** `[stage, ms]`: when each stage was reached. */
  stages: [string, number][];
  /** One line per reason the tier is what it is (the first is the model's). */
  tier_reasons?: string[];
  /** Why it ended; for a released one, the parts and versions it shipped in. */
  ended_reason?: string;
  from?: { kind: string; name: string; node: string };
}

/** The board's four columns, in a person's words. */
export type Column = 'incoming' | 'waiting' | 'coding' | 'done' | 'shipped';

export const COLUMNS: Array<{ key: Column; label: string }> = [
  { key: 'incoming', label: 'Incoming' },
  { key: 'waiting', label: 'Waiting' },
  { key: 'coding', label: 'Coding' },
  { key: 'done', label: 'Done' },
  { key: 'shipped', label: 'Shipped' },
];

/** A request as the board shows it (`BoardItem`, flattened). */
export interface BoardItem {
  id: string;
  repo: string;
  session_id: string;
  title: string;
  kind: string;
  tier: Tier;
  surface: string[];
  urgent: boolean;
  stage: string;
  ended_as: string;
  package: string;
  from?: { kind: string; name: string; node: string };
  word: Word;
  column: Column;
  paused: boolean;
  starts_ms: number;
  step: number;
  steps: number;
  checking: boolean;
  /** Started but not building yet: next | after | builder. */
  queued: string;
  soak_until_ms: number;
  release_ms: number;
  released_ms: number;
  /** The package it is built in with others; empty when it is alone. */
  together: string;
  reports: number;
  reporters: string[];
  /** The next release waits for it. */
  must_ship?: boolean;
  follows?: string;
}

export interface Finding {
  title: string;
  evidence: string;
}

export interface Gate {
  /** scope | standard | charter | build | walks | converge | human. */
  kind: string;
  attempt: number;
  /** passed | failed | blocked | not_run. */
  result: string;
  findings: Finding[];
  ms: number;
}

export interface Evidence {
  id: string;
  walk: string;
  request: string;
  /** shot | recording | log | trace. */
  kind: string;
  /** passed | failed | blocked | not_run. */
  result: string;
  seen: string;
  build: string;
  ms: number;
}

export interface Intervention {
  /** W1..W8 | drift | replan | approval. */
  rule: string;
  fact: string;
  /** steer | restart | stop | refuse | note. */
  action: string;
  text: string;
  task: number;
  ms: number;
}

export interface Strip {
  task: number;
  tasks: number;
  attempt: number;
  checkpoint_ms: number;
  files: number;
  lines: number;
  limit_lines: number;
  in_scope: boolean;
  minutes: number;
  limit_minutes: number;
  steered: number;
  restarted: number;
  now: string;
  /** `[rule, fired]`. */
  rules: [string, boolean][];
}

/** A package with what is computed about it (`PackageView`, flattened). */
export interface PackageView {
  id: string;
  repo: string;
  requests: string[];
  tier: Tier;
  urgent: boolean;
  stage: string;
  /** 0 for a major package: its hold ends by a yes. */
  hold_until_ms: number;
  held_since_ms: number;
  branch: string;
  base: string;
  attempt: number;
  reworks: number;
  parts: string[];
  job_id: string;
  session_id: string;
  landed: string;
  landed_ms: number;
  soak_until_ms: number;
  stopped: boolean;
  created_ms: number;
  /** The computer building it, by name. */
  runner_name?: string;
  word: Word;
  points: Point[];
  gates: Gate[];
  interventions: Intervention[];
  evidence: Evidence[];
  strip?: Strip;
  digest: string;
  budget: Budget;
}

export interface Ask {
  id: string;
  kind: string;
  repo: string;
  request: string;
  package: string;
  title: string;
  subject: string;
  why: string;
  if_yes: string;
  if_no: string;
  actions: string[];
  attempt: number;
  since_ms: number;
}

export interface Going {
  package: string;
  repo: string;
  title: string;
  line: string;
  hold_until_ms: number;
  urgent: boolean;
}

export interface Inbox {
  asks: Ask[];
  going: Going[];
  count_line: string;
}

export interface WatchBeat {
  package: string;
  strip: Strip;
  digest: string;
}

/** Published after any change; every list here reloads on it. */
export const CHANGED = 'manage.changed';
/** A `WatchBeat` on every beat that changed something a person sees. */
export const WATCH = 'manage.watch';

/** The stages in order, as `stage::ALL` of the contract has them. */
export const STAGES = ['arrived', 'clarifying', 'assessed', 'planned', 'validated', 'held', 'building', 'proving', 'ready', 'landed', 'soaking', 'released'] as const;

export const STAGE_LABEL: Record<string, string> = {
  arrived: 'Arrived',
  clarifying: 'Clarifying',
  assessed: 'Assessed',
  planned: 'Planned',
  validated: 'Validated',
  held: 'Held',
  building: 'Building',
  proving: 'Proving',
  ready: 'Ready to ship',
  landed: 'Landed',
  soaking: 'Waiting for release',
  released: 'Released',
};

/** The stage in one short word, for a sidebar row. */
const STAGE_SHORT: Record<string, string> = {
  arrived: 'New',
  clarifying: 'Asks',
  assessed: 'Assessed',
  planned: 'Plan',
  validated: 'Checked',
  held: 'Held',
  building: 'Build',
  proving: 'Prove',
  ready: 'Ready',
  landed: 'Landed',
  soaking: 'Done',
  released: 'Released',
};

export const WORD_LABEL: Record<Word, string> = {
  on_hold: 'On hold',
  needs_you: 'Needs you',
  stuck: 'Stuck',
  rework: 'Rework',
  working: 'Working',
  watching: 'Watching',
  held: 'Held',
  queued: 'Queued',
  done: 'Done',
  idle: 'Idle',
};

/** What a sidebar row says at its right: the stage, unless the request wants the person. */
export function shortStage(request: Pick<RequestView, 'stage' | 'word'>): string {
  if (request.word === 'needs_you' || request.word === 'on_hold' || request.word === 'stuck' || request.word === 'rework') return WORD_LABEL[request.word];
  return STAGE_SHORT[request.stage] ?? request.stage;
}

/**
 * The theme's 5px state square for a word: at work pulses in the counter colour, what needs the
 * person is a solid rubric square, done is the third ink, stuck is outlined in the deep rubric,
 * and what only waits is hollow.
 */
export function pipClass(word: Word | undefined): string {
  switch (word) {
    case 'working':
    case 'watching':
    case 'rework':
      return 'cx-pip cx-pip-working';
    case 'needs_you':
    case 'on_hold':
      return 'cx-pip cx-pip-waiting';
    case 'done':
      return 'cx-pip cx-pip-done';
    case 'stuck':
      return 'cx-pip cx-pip-failed';
    default:
      return 'cx-pip';
  }
}

/** The square for a gate's or a walk's result. Blocked and not run are hollow: not proven. */
export function resultPip(result: string): string {
  if (result === 'passed') return 'cx-pip cx-pip-done';
  if (result === 'failed') return 'cx-pip cx-pip-waiting';
  return 'cx-pip';
}

export const TIER_LABEL: Record<Tier, string> = { safe: 'Safe', minor: 'Minor', major: 'Major' };

// ─── the selection ──────────────────────────────────────────────────────────

export type View = 'session' | 'new' | 'inbox' | 'charter' | 'releases' | 'runner';

/** The sidebar unfolded over the work area (the workbench's shared key). */
export const LAYOUT_BOARD = 'layout.board';

/** What is open, for every package of the window (shared key `manage.selection`). */
export interface Selection {
  view: View;
  repo: string | null;
  request: string | null;
  package: string | null;
}

export const NO_SELECTION: Selection = { view: 'inbox', repo: null, request: null, package: null };

export function useManageSelection(): [Selection, (next: Selection) => void] {
  const [selection, setSelection] = useShared<Selection>('manage.selection', NO_SELECTION);
  return [selection ?? NO_SELECTION, setSelection];
}

/** The selection that opens a request's session. */
export function sessionOf(request: Pick<RequestView, 'id' | 'repo' | 'package'>): Selection {
  return { view: 'session', repo: request.repo, request: request.id, package: request.package || null };
}

/** The open request and its package, for the right pane's tabs (`enabled`: the tab is in front). */
export function useOpen(enabled = true): { selection: Selection; request: QueryResult<RequestView>; pack: QueryResult<PackageView>; packageId: string | null } {
  const [selection] = useManageSelection();
  const requestId = selection.view === 'session' ? selection.request : null;
  const request = useQuery<RequestView>(
    enabled && requestId ? `manage.request.get:${requestId}` : null,
    (client) => client.call<RequestView>('manage.request.get', { id: requestId }),
    { topics: [CHANGED] },
  );
  // The request knows its package as of now; the selection's is from when the row was pressed.
  const packageId = requestId ? (request.data ? request.data.package || null : selection.package) : null;
  const pack = useQuery<PackageView>(
    enabled && packageId ? `manage.package.get:${packageId}` : null,
    (client) => client.call<PackageView>('manage.package.get', { id: packageId }),
    { topics: [CHANGED] },
  );
  return { selection, request, pack, packageId };
}

// ─── gates ──────────────────────────────────────────────────────────────────

export const GATE_LABEL: Record<string, string> = {
  scope: 'Scope',
  standard: 'Standard',
  charter: 'Charter',
  build: 'Proving build',
  walks: 'Walks in the real app',
  converge: 'Converge',
  human: 'Your yes',
};

export const RESULT_LABEL: Record<string, string> = {
  passed: 'Passed',
  failed: 'Failed',
  blocked: 'Blocked',
  not_run: 'Not run',
};

const GATE_ORDER = ['scope', 'standard', 'charter', 'build', 'walks', 'converge', 'human'];

/** The newest answer of each kind of gate, in the order a package passes them. */
export function latestGates(gates: Gate[]): Gate[] {
  const newest = new Map<string, Gate>();
  for (const gate of gates) {
    const current = newest.get(gate.kind);
    if (!current || gate.attempt > current.attempt || (gate.attempt === current.attempt && gate.ms >= current.ms)) newest.set(gate.kind, gate);
  }
  const rank = (kind: string) => {
    const index = GATE_ORDER.indexOf(kind);
    return index < 0 ? GATE_ORDER.length : index;
  };
  return [...newest.values()].sort((a, b) => rank(a.kind) - rank(b.kind));
}

// ─── times ──────────────────────────────────────────────────────────────────

const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const two = (value: number) => String(value).padStart(2, '0');

/** `14:06` today, `Mon 14:06` on another day; nothing for a time that is not set. */
export function timeOf(ms: number, now = Date.now()): string {
  if (!ms) return '';
  const date = new Date(ms);
  const clock = `${two(date.getHours())}:${two(date.getMinutes())}`;
  return date.toDateString() === new Date(now).toDateString() ? clock : `${DAYS[date.getDay()]} ${clock}`;
}

/** A length of time in one unit: `25 m`, `2 h`, `3 d`. */
export function span(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} m`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} h`;
  return `${Math.round(hours / 24)} d`;
}

/** `waiting 2 h`. */
export function waiting(sinceMs: number, now = Date.now()): string {
  return `waiting ${span(Math.max(0, now - sinceMs))}`;
}

/** A number of minutes as a policy states it: `90 min`, `12 h`, `2 d`. */
export function minutesText(minutes: number): string {
  if (minutes < 120 || minutes % 60 !== 0) return `${minutes} min`;
  const hours = minutes / 60;
  return hours >= 48 && hours % 24 === 0 ? `${hours / 24} d` : `${hours} h`;
}

export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

// ─── failures, said plainly ─────────────────────────────────────────────────

function fullStop(text: string): string {
  const trimmed = text.trim();
  return /[.!?]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

/** One sentence for a call that failed: what was being done, and what the service said. */
export function failure(doing: string, cause: unknown): string {
  if (cause instanceof RpcError && cause.code === 'unavailable') return `${doing} did not work: the manage service is not running.`;
  return fullStop(`${doing} did not work: ${errorMessage(cause)}`);
}

/** One sentence for a list that could not be loaded. */
export function loadFailure(what: string, state: { error?: string | undefined; errorCode?: string | undefined }): string {
  if (state.errorCode === 'unavailable') return `${what} cannot be shown: the manage service is not running.`;
  return fullStop(`${what} could not be loaded: ${state.error ?? 'no reason was given'}`);
}

export interface Act {
  /** The key of the action that is running, if one is. */
  busy: string | null;
  /** The sentence for the action `key` when it failed, to draw next to its control. */
  failed(key: string): string | null;
  /** Run `work`; a failure becomes `doing did not work: …` under `key`. True when it worked. */
  run(key: string, doing: string, work: () => Promise<unknown>): Promise<boolean>;
}

/** Runs the calls of one region, one at a time, and keeps the last failure with its control. */
export function useAct(): Act {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ key: string; text: string } | null>(null);
  const run = useCallback(async (key: string, doing: string, work: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await work();
      return true;
    } catch (cause) {
      setError({ key, text: failure(doing, cause) });
      return false;
    } finally {
      setBusy(null);
    }
  }, []);
  const failed = useCallback((key: string) => (error && error.key === key ? error.text : null), [error]);
  return { busy, failed, run };
}

// ─── this computer ──────────────────────────────────────────────────────────

/**
 * The manage plugin runs on this computer, whichever computer's project the workbench shows, so
 * every package of this window talks to this computer's frame.
 */
export function local<P extends object>(Inner: ComponentType<P>): (props: P) => ReactElement {
  return function Local(props: P) {
    return createElement(ComputerScope, { node: null }, createElement(Inner, props));
  };
}
