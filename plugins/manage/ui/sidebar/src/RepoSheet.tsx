/**
 * A repository's own pop-up, opened from the board: its name and git address, who its commits are
 * made as, which computer builds it, when it releases, and (under Advanced) how long work waits and
 * runs in the apps and the paths that make a change major. Nothing is saved until Save.
 */

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Menu, useClient, useQuery, useSlotInfo } from '@cortex/sdk';
import { useAct, type Budget, type Policy, type Repo, type Tier } from './manage';
import './repo.css';

/** The policy's whole numbers, with what each is called when it is wrong. */
const NUMBERS = [
  ['hold_safe_min', 'How long a safe change waits'],
  ['hold_minor_min', 'How long a minor change waits'],
  ['hold_max_min', 'The longest wait'],
  ['soak_safe_min', 'How long a safe change runs in your apps'],
  ['soak_minor_min', 'How long a minor change runs in your apps'],
  ['soak_major_min', 'How long a major change runs in your apps'],
  ['rework_safe', 'The rework rounds for safe changes'],
  ['rework_minor', 'The rework rounds for minor changes'],
  ['rework_major', 'The rework rounds for major changes'],
] as const;
type NumberKey = (typeof NUMBERS)[number][0];

const TIERS: Tier[] = ['safe', 'minor', 'major'];
const BUDGET_PARTS = ['minutes', 'tokens', 'lines'] as const;

/** What the form holds while it is edited: numbers as they are typed, path lists one per line. */
interface Draft {
  name: string;
  url: string;
  main: string;
  changes: string;
  commitName: string;
  commitEmail: string;
  github: string;
  runner: string;
  runner_name: string;
  managed: boolean;
  numbers: Record<NumberKey, string>;
  budgets: Record<Tier, [string, string, string]>;
  train_at: string;
  train_every: string;
  train_day: number;
  major_paths: string;
  minor_paths: string;
}

function draftOf(repo: Repo): Draft {
  const policy = repo.policy;
  const numbers = {} as Record<NumberKey, string>;
  for (const [key] of NUMBERS) numbers[key] = String(policy[key]);
  const budget = (value: Budget): [string, string, string] => [String(value[0]), String(value[1]), String(value[2])];
  return {
    name: repo.name,
    url: repo.source.url,
    main: repo.source.main,
    changes: repo.source.changes || 'none',
    commitName: repo.source.commit_name ?? '',
    commitEmail: repo.source.commit_email ?? '',
    github: repo.source.github ?? '',
    runner: repo.runner,
    runner_name: repo.runner_name,
    managed: repo.managed,
    numbers,
    budgets: { safe: budget(policy.budget_safe), minor: budget(policy.budget_minor), major: budget(policy.budget_major) },
    train_at: policy.train_at,
    train_every: policy.train_every ?? '',
    train_day: policy.train_day ?? 0,
    major_paths: policy.major_paths.join('\n'),
    minor_paths: policy.minor_paths.join('\n'),
  };
}

function whole(text: string): number | null {
  return /^\d+$/.test(text.trim()) ? Number(text.trim()) : null;
}

function lines(text: string): string[] {
  return text.split('\n').map((line) => line.trim()).filter(Boolean);
}

/** The policy the draft states, or the sentence saying what in it cannot be saved. */
function policyOf(draft: Draft): Policy | string {
  const numbers = {} as Record<NumberKey, number>;
  for (const [key, called] of NUMBERS) {
    const value = whole(draft.numbers[key]);
    if (value === null) return `${called} must be a number of hours.`;
    numbers[key] = value;
  }
  const budgets = {} as Record<Tier, Budget>;
  for (const tier of TIERS) {
    const values = draft.budgets[tier].map(whole);
    const wrong = values.findIndex((value) => value === null);
    if (wrong >= 0) return `The budget's ${BUDGET_PARTS[wrong]} for ${tier} changes must be a number of hours.`;
    budgets[tier] = values as Budget;
  }
  const clock = /^(\d{2}):(\d{2})$/.exec(draft.train_at.trim());
  if (!clock || Number(clock[1]) > 23 || Number(clock[2]) > 59) return 'The release time needs to look like 03:00.';
  return {
    ...numbers,
    budget_safe: budgets.safe,
    budget_minor: budgets.minor,
    budget_major: budgets.major,
    major_paths: lines(draft.major_paths),
    minor_paths: lines(draft.minor_paths),
    train_at: draft.train_at.trim(),
    train_every: draft.train_every,
    train_day: draft.train_day,
  };
}

/** A joined computer as `mesh.peers` answers it (the fields the runner menu reads). */
interface Peer { member: { node: string; name: string }; card?: { software?: string } | null; reachable: boolean }

/** A time the policy keeps in minutes, shown and typed in hours. */
function Hours({ value, label, onChange }: { value: string; label: string; onChange: (minutes: string) => void }) {
  const minutes = Number(value);
  const shown = /^\d+$/.test(value.trim()) ? String(Math.round((minutes / 60) * 100) / 100) : value;
  return (
    <span className="cm-pol-field">
      <label className="field n">
        <input
          value={shown}
          inputMode="decimal"
          aria-label={`${label}, in hours`}
          onChange={(event) => {
            const hours = Number(event.target.value.trim());
            onChange(event.target.value.trim() !== '' && Number.isFinite(hours) && hours >= 0 ? String(Math.round(hours * 60)) : event.target.value);
          }}
        />
      </label>
      <span className="ap faint">h</span>
    </span>
  );
}

function RepoSettings({ repo, saved }: { repo: Repo; saved: () => void }) {
  const client = useClient();
  const act = useAct();
  // Only a computer running CortexCode can build: its card says which app it is.
  const peers = useQuery<Peer[]>('mesh.peers:runners', async (c) => {
    const all = (await c.call<{ peers: Peer[] }>('mesh.peers', {}))?.peers ?? [];
    return all.filter((peer) => peer.card?.software === 'cortexcode');
  }, { topics: ['mesh.changed'] });
  const base = draftOf(repo);
  const baseText = JSON.stringify(base);
  const [draft, setDraft] = useState<Draft>(base);
  const [synced, setSynced] = useState(baseText);
  const [invalid, setInvalid] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);
  // The repository changed elsewhere: an untouched form follows it, an edited one keeps its edits.
  if (synced !== baseText) {
    if (JSON.stringify(draft) === synced) setDraft(base);
    setSynced(baseText);
  }
  const dirty = JSON.stringify(draft) !== baseText;
  const set = (patch: Partial<Draft>) => {
    setInvalid(null);
    setDraft((current) => ({ ...current, ...patch }));
  };
  const setNumber = (key: NumberKey, value: string) => set({ numbers: { ...draft.numbers, [key]: value } });

  const save = () => {
    if (!draft.name.trim()) return setInvalid('The repository needs a name.');
    if (!draft.url.trim()) return setInvalid('The repository needs its git URL.');
    if (!draft.main.trim()) return setInvalid('The repository needs its main branch.');
    const policy = policyOf(draft);
    if (typeof policy === 'string') return setInvalid(policy);
    setInvalid(null);
    void act.run('save', `Saving ${repo.name}`, async () => {
      const after = await client.call<Repo>('manage.repo.edit', {
        id: repo.id,
        name: draft.name.trim(),
        source: { kind: repo.source.kind || 'git', url: draft.url.trim(), main: draft.main.trim(), changes: draft.changes, commit_name: draft.commitName.trim(), commit_email: draft.commitEmail.trim(), github: draft.github.trim() },
        runner: draft.runner,
        runner_name: draft.runner_name,
        policy,
        managed: draft.managed,
      });
      // The form shows what was kept, which may be stricter than what was typed.
      if (after && after.id === repo.id) {
        const kept = draftOf(after);
        setDraft(kept);
        setSynced(JSON.stringify(kept));
      }
      saved();
    });
  };

  const runnerItems = [
    { key: '', label: 'Any free CortexCode', selected: draft.runner === '', onSelect: () => set({ runner: '', runner_name: '' }) },
    ...(peers.data ?? []).map((peer) => ({
      key: peer.member.node,
      label: peer.member.name,
      note: peer.reachable ? 'CortexCode' : 'not reachable now',
      selected: draft.runner === peer.member.node,
      onSelect: () => set({ runner: peer.member.node, runner_name: peer.member.name }),
    })),
  ];
  const failed = invalid ?? act.failed('save');

  return (
    <div className="cm-set-grid" data-repo={repo.id}>
      <span className="ap">Name</span>
      <span className="cm-set-cell">
        <label className="field"><input value={draft.name} aria-label="Name" onChange={(event) => set({ name: event.target.value })} /></label>
      </span>

      <span className="ap">Git URL</span>
      <span className="cm-set-cell"><label className="field"><input value={draft.url} aria-label="Git URL" onChange={(event) => set({ url: event.target.value })} /></label></span>
      <span className="ap">GitHub</span>
      <span className="cm-set-cell"><label className="field"><input value={draft.github} placeholder="owner/name" aria-label="GitHub repository, as owner/name" onChange={(event) => set({ github: event.target.value })} /></label></span>
      <span className="ap">Main branch</span>
      <span className="cm-set-cell"><label className="field"><input value={draft.main} aria-label="Main branch" onChange={(event) => set({ main: event.target.value })} /></label></span>
      <span className="ap">Commits as</span>
      <span className="cm-set-cell">
        <label className="field"><input value={draft.commitName} placeholder="Name" aria-label="Name the commits are made as" onChange={(event) => set({ commitName: event.target.value })} /></label>
        <label className="field"><input value={draft.commitEmail} placeholder="Email" aria-label="Email the commits are made as" onChange={(event) => set({ commitEmail: event.target.value })} /></label>
      </span>

      <span className="ap">Builds on</span>
      <span className="cm-set-cell">
        <Menu label="Computer" value={draft.runner ? draft.runner_name || draft.runner : 'any free CortexCode'} items={runnerItems} />
        {peers.error && !peers.data && (
          <span className="cm-set-note">The other computers cannot be listed: {peers.errorCode === 'unavailable' ? 'the mesh is not running' : peers.error}.</span>
        )}
      </span>

      <span className="ap">Releases</span>
      <span className="cm-set-cell">
        <span className="seg-set" role="group" aria-label="How often a release is made">
          <button type="button" aria-pressed={draft.train_every === '3h'} onClick={() => set({ train_every: '3h' })}>Every 3 hours</button>
          <button type="button" aria-pressed={draft.train_every !== '3h' && draft.train_every !== 'week'} onClick={() => set({ train_every: '' })}>Daily</button>
          <button type="button" aria-pressed={draft.train_every === 'week'} onClick={() => set({ train_every: 'week' })}>Weekly</button>
        </span>
        <span className="ap faint">{draft.train_every === '3h' ? 'from' : 'at'}</span>
        <label className="field t"><input value={draft.train_at} aria-label="Release time, as HH:MM" placeholder="03:00" onChange={(event) => set({ train_at: event.target.value })} /></label>
      </span>
      {draft.train_every === 'week' && (
        <>
          <span />
          <span className="cm-set-cell">
            <span className="seg-set" role="group" aria-label="Which day">
              {['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((day, index) => (
                <button key={day} type="button" aria-pressed={draft.train_day === index} onClick={() => set({ train_day: index })}>{day}</button>
              ))}
            </span>
          </span>
        </>
      )}

      <span />
      <span className="cm-set-cell">
        <button type="button" className="detent" aria-pressed={advanced} aria-expanded={advanced} onClick={() => setAdvanced(!advanced)}>Advanced</button>
      </span>
      {advanced && (
        <>
      <span className="ap">A landed package</span>
      <span className="cm-set-cell">
        <span className="seg-set" role="group" aria-label="What a landed package also does">
          <button type="button" aria-pressed={draft.changes !== 'github-pr'} onClick={() => set({ changes: 'none' })}>Main branch only</button>
          <button type="button" aria-pressed={draft.changes === 'github-pr'} onClick={() => set({ changes: 'github-pr' })}>Also a pull request</button>
        </span>
      </span>

      <span className="ap cm-set-h">Waiting before coding</span>
      <div className="cm-pol">
        <span />
        <span className="ap faint">Safe</span>
        <span className="ap faint">Minor</span>
        <span className="ap faint">Major</span>
        <span className="ap">Waits</span>
        <Hours value={draft.numbers.hold_safe_min} label="A safe change waits" onChange={(value) => setNumber('hold_safe_min', value)} />
        <Hours value={draft.numbers.hold_minor_min} label="A minor change waits" onChange={(value) => setNumber('hold_minor_min', value)} />
        <span className="cm-pol-note">until your yes</span>
      </div>
        </>
      )}

      <span className="cm-set-foot">
        <button type="button" className="detent send" aria-pressed={dirty} disabled={!dirty || act.busy !== null} onClick={save}>Save</button>
        <button type="button" className="detent" disabled={!dirty || act.busy !== null} onClick={() => { setInvalid(null); setDraft(base); }}>Revert</button>
        {failed && <p className="inline-error">{failed}</p>}
      </span>
    </div>
  );
}


/** The pop-up around the form; Escape or Close leaves it. */
export function RepoSheet({ repo, onClose }: { repo: Repo; onClose: () => void }) {
  const owner = useSlotInfo();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented) close.current();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
  const root = document.getElementById('cc') ?? document.body;
  return createPortal(
    <div className="sheet-scrim open" data-slot={owner.slot} data-plugin-id={owner.pluginId} data-portal="repo">
      <div className="sheet cm-repo-sheet" role="dialog" aria-modal="true" aria-label={repo.name}>
        <div className="sheet-h">
          <span className="ap">{repo.name}</span>
          <span className="grow" />
          <button className="detent" type="button" onClick={onClose}>Close</button>
        </div>
        <div className="sheet-b">
          <RepoSettings key={repo.id} repo={repo} saved={onClose} />
        </div>
      </div>
    </div>,
    root,
  );
}
