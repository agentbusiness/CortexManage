/** The shapes of `charter@1` this page reads (protocol/src/contracts/charter.rs). */

export type Level = 'law' | 'constraint' | 'convention';
export type KeptBy = 'check' | 'validator' | 'text';

export interface Rule {
  id: string;
  level: Level;
  /** `standard` or the repository's id. */
  scope: string;
  text: string;
  why?: string;
  applies_to?: string[];
  kept_by?: KeptBy | '';
  check?: string;
  since?: string;
  amended_by?: string;
  source?: string;
}

export interface Identity {
  what?: string;
  who_for?: string;
  never?: string[];
}

export interface Charter {
  repo: string;
  version: string;
  draft?: boolean;
  identity?: Identity;
  rules: Rule[];
  /** The standard charter's laws and constraints, for an app's charter. */
  standard?: Rule[];
  standard_version?: string;
  digest?: string;
  updated_ms?: number;
}

export interface CharterRow { repo: string; version: string; rules: number; draft: boolean }

export interface Proposal {
  repo: string;
  /** Empty for a new rule. */
  id: string;
  level: Level;
  old: string;
  new: string;
  why?: string;
  /** `law`: only the owner, by hand. `major` | `minor`: the tier of the request that changes it. */
  needs: 'law' | 'major' | 'minor';
  conflicts?: string[];
  digest: string;
}

export interface Printed { repo: string; version: string; text: string; digest: string; head_chars: number }

/** The workbench's shared selection (`manage.selection`). */
export interface Selection {
  view: 'session' | 'inbox' | 'charter' | 'releases' | 'runner' | 'standard';
  repo: string | null;
  request: string | null;
  package: string | null;
}

/** The standard charter's repository id. */
export const STANDARD = 'standard';
/** How many characters of AGENTS.md CortexCode's model reads. */
export const HEAD_LIMIT = 6000;

export const LEVELS: Array<{ value: Level; label: string; plural: string; changedBy: string }> = [
  { value: 'law', label: 'Law', plural: 'Laws', changedBy: 'only you, by hand' },
  { value: 'constraint', label: 'Constraint', plural: 'Constraints', changedBy: 'a major change' },
  { value: 'convention', label: 'Convention', plural: 'Conventions', changedBy: 'a minor change' },
];

export const KEPT: Array<{ value: KeptBy; label: string }> = [
  { value: 'check', label: 'Check' },
  { value: 'validator', label: 'Validator' },
  { value: 'text', label: 'Text only' },
];
