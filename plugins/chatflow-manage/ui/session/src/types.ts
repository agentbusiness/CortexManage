/** The shapes of `manage@1` this view reads (protocol/src/contracts/manage.rs). */

export interface Selection {
  view: 'session' | 'inbox' | 'charter' | 'releases' | 'runner' | 'standard';
  repo: string | null;
  request: string | null;
  package: string | null;
}

export const NO_SELECTION: Selection = { view: 'inbox', repo: null, request: null, package: null };

export interface Question {
  id: string;
  text: string;
  options?: string[];
  multi?: boolean;
  answer?: string[];
}

export interface Walk {
  id: string;
  do: string;
  see: string;
  method: string;
  evidence_kind?: string;
  command?: string;
  expect?: string;
}

export interface Point {
  n: number;
  title: string;
  brief?: string;
  rules?: string[];
  touches?: string[];
  walks?: Walk[];
  verdict?: string;
  verdict_reason?: string;
}

export interface Plan {
  version: number;
  goal?: string;
  architecture?: string;
  points: Point[];
  note?: string;
}

export interface RequestView {
  id: string;
  repo: string;
  session_id: string;
  title: string;
  text: string;
  kind?: string;
  tier?: string;
  surface?: string[];
  urgent?: boolean;
  stage: string;
  ended_as?: string;
  ended_reason?: string;
  package?: string;
  questions?: Question[];
  word?: string;
}

export interface Ask {
  id: string;
  kind: string;
  title: string;
  subject: string;
  why: string;
  if_yes?: string;
  if_no?: string;
  actions: string[];
  attempt: number;
}

export interface Finding {
  title: string;
  evidence?: string;
}

/** The stages before a job builds: the request can still be made urgent, its package started or stopped. */
export const BEFORE_BUILDING = ['arrived', 'clarifying', 'assessed', 'planned', 'validated', 'held'];

/** The word a person sees for a request (`Word`), in plain letters. */
export function wordLabel(word: string | undefined): string {
  return (word ?? '').replace(/_/g, ' ');
}
