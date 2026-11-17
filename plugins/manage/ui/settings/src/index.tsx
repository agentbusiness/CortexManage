/**
 * settings.manage@1 — Settings > Repositories. For the repository chosen: where its main line
 * lives, which computer builds it, the policy's numbers (hold, soak, rework rounds, budgets, the
 * daily train) and its path lists, and whether it is managed at all. Nothing is saved until Save.
 */

import { useState } from 'react';
import { Menu, definePlugin, useClient, useQuery } from '@cortex/sdk';
import { CHANGED, loadFailure, local, useAct, useManageSelection, type Budget, type Policy, type Repo, type Tier } from './manage';
import './manage.css';
import './settings.css';

/** The policy's whole numbers, with what each is called when it is wrong. */
const NUMBERS = [
  ['hold_safe_min', 'The hold for safe changes'],
  ['hold_minor_min', 'The hold for minor changes'],
  ['hold_max_min', 'The longest hold'],
  ['soak_safe_min', 'The soak for safe changes'],
  ['soak_minor_min', 'The soak for minor changes'],
  ['soak_major_min', 'The soak for major changes'],
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
  runner: string;
  runner_name: string;
  managed: boolean;
  numbers: Record<NumberKey, string>;
  budgets: Record<Tier, [string, string, string]>;
  train_at: string;
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
    runner: repo.runner,
    runner_name: repo.runner_name,
    managed: repo.managed,
    numbers,
    budgets: { safe: budget(policy.budget_safe), minor: budget(policy.budget_minor), major: budget(policy.budget_major) },
    train_at: policy.train_at,
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
    if (value === null) return `${called} must be a whole number.`;
    numbers[key] = value;
  }
  const budgets = {} as Record<Tier, Budget>;
  for (const tier of TIERS) {
    const values = draft.budgets[tier].map(whole);
    const wrong = values.findIndex((value) => value === null);
    if (wrong >= 0) return `The budget's ${BUDGET_PARTS[wrong]} for ${tier} changes must be a whole number.`;
    budgets[tier] = values as Budget;
  }
  const clock = /^(\d{2}):(\d{2})$/.exec(draft.train_at.trim());
  if (!clock || Number(clock[1]) > 23 || Number(clock[2]) > 59) return 'The daily train needs a time like 03:00.';
  return {
    ...numbers,
    budget_safe: budgets.safe,
    budget_minor: budgets.minor,
    budget_major: budgets.major,
    major_paths: lines(draft.major_paths),
    minor_paths: lines(draft.minor_paths),
    train_at: draft.train_at.trim(),
  };
}

/** A joined computer as `mesh.peers` answers it (the fields the runner menu reads). */
interface Peer { member: { node: string; name: string }; card?: { software?: string } | null; reachable: boolean }

function NumberField({ value, label, unit, onChange }: { value: string; label: string; unit?: string; onChange: (next: string) => void }) {
  return (
    <span className="cm-set-pair">
      <span className="ap faint">{label}</span>
      <label className="field n"><input value={value} inputMode="numeric" aria-label={label} onChange={(event) => onChange(event.target.value)} /></label>
      {unit && <span className="ap faint">{unit}</span>}
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
  const setBudget = (tier: Tier, index: number, value: string) => {
    const next: [string, string, string] = [...draft.budgets[tier]];
    next[index] = value;
    set({ budgets: { ...draft.budgets, [tier]: next } });
  };

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
        source: { kind: repo.source.kind || 'git', url: draft.url.trim(), main: draft.main.trim(), changes: draft.changes, commit_name: draft.commitName.trim(), commit_email: draft.commitEmail.trim() },
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
        <span className="ap faint">Requests {repo.prefix}-0001…</span>
      </span>

      <span className="ap">Git URL</span>
      <span className="cm-set-cell"><label className="field"><input value={draft.url} aria-label="Git URL" onChange={(event) => set({ url: event.target.value })} /></label></span>
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

      <span />
      <span className="cm-set-cell">
        <button type="button" className="detent" aria-pressed={advanced} aria-expanded={advanced} onClick={() => setAdvanced(!advanced)}>Advanced</button>
        {!advanced && <span className="cm-set-note">Hold and soak times, rework rounds, budgets, the daily train and the paths that make a change major. The defaults work.</span>}
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

      <span className="ap cm-set-h">Policy</span>
      <span className="ap">Hold</span>
      <span className="cm-set-cell">
        <NumberField label="Safe" unit="min" value={draft.numbers.hold_safe_min} onChange={(value) => setNumber('hold_safe_min', value)} />
        <NumberField label="Minor" unit="min" value={draft.numbers.hold_minor_min} onChange={(value) => setNumber('hold_minor_min', value)} />
        <NumberField label="At most" unit="min" value={draft.numbers.hold_max_min} onChange={(value) => setNumber('hold_max_min', value)} />
      </span>
      <span className="ap">Soak</span>
      <span className="cm-set-cell">
        <NumberField label="Safe" unit="min" value={draft.numbers.soak_safe_min} onChange={(value) => setNumber('soak_safe_min', value)} />
        <NumberField label="Minor" unit="min" value={draft.numbers.soak_minor_min} onChange={(value) => setNumber('soak_minor_min', value)} />
        <NumberField label="Major" unit="min" value={draft.numbers.soak_major_min} onChange={(value) => setNumber('soak_major_min', value)} />
      </span>
      <span className="ap">Rework rounds</span>
      <span className="cm-set-cell">
        <NumberField label="Safe" value={draft.numbers.rework_safe} onChange={(value) => setNumber('rework_safe', value)} />
        <NumberField label="Minor" value={draft.numbers.rework_minor} onChange={(value) => setNumber('rework_minor', value)} />
        <NumberField label="Major" value={draft.numbers.rework_major} onChange={(value) => setNumber('rework_major', value)} />
      </span>
      <BudgetRow values={draft.budgets.major} onChange={(index, value) => setBudget('major', index, value)} />
      <span className="ap">Daily train</span>
      <span className="cm-set-cell">
        <label className="field t"><input value={draft.train_at} aria-label="Daily train, as HH:MM" placeholder="03:00" onChange={(event) => set({ train_at: event.target.value })} /></label>
        <span className="cm-set-note">Every day at this time, what has soaked is released.</span>
      </span>

      <span className="ap cm-set-h">Paths</span>
      <span className="ap">Major</span>
      <span className="cm-set-cell">
        <textarea className="cm-area" value={draft.major_paths} aria-label="Paths that make a change major, one per line" spellCheck={false} onChange={(event) => set({ major_paths: event.target.value })} />
        <span className="cm-set-note">One path pattern per line. A change under any of them is major. Every file of the mesh standard is major as well.</span>
      </span>
      <span className="ap">At least minor</span>
      <span className="cm-set-cell">
        <textarea className="cm-area" value={draft.minor_paths} aria-label="Paths that make a change at least minor, one per line" spellCheck={false} onChange={(event) => set({ minor_paths: event.target.value })} />
      </span>

        </>
      )}

      <span />
      <span className="cm-set-foot">
        <button type="button" className="detent send" aria-pressed={dirty} disabled={!dirty || act.busy !== null} onClick={save}>Save</button>
        <button type="button" className="detent" disabled={!dirty || act.busy !== null} onClick={() => { setInvalid(null); setDraft(base); }}>Revert</button>
        {failed && <p className="inline-error">{failed}</p>}
      </span>
    </div>
  );
}

/** The runaway guard: the longest any job may run and the most tokens it may use. */
function BudgetRow({ values, onChange }: { values: [string, string, string]; onChange: (index: number, value: string) => void }) {
  return (
    <>
      <span className="ap">A job stops after</span>
      <span className="cm-set-cell">
        <NumberField label="Minutes" value={values[0]} onChange={(value) => onChange(0, value)} />
        <NumberField label="Tokens" value={values[1]} onChange={(value) => onChange(1, value)} />
      </span>
    </>
  );
}

function ManageSettings() {
  const repos = useQuery<Repo[]>('manage.repo.list', async (client) => (await client.call<{ repos: Repo[] }>('manage.repo.list', {}))?.repos ?? [], { topics: [CHANGED] });
  const [selection] = useManageSelection();
  const [chosen, setChosen] = useState<string | null>(selection.repo);
  const list = repos.data ?? [];
  const repo = list.find((candidate) => candidate.id === chosen) ?? list[0];
  return (
    <section className="cm-set" data-build={__CORTEX_BUILD__}>
      {repos.error && !repos.data && <p className="inline-error">{loadFailure('The repositories', repos)}</p>}
      {repos.data && list.length === 0 && <p className="cm-line flush">No repository yet. Add one with Add repo, at the foot of the sidebar.</p>}
      {list.length > 1 && (
        <div className="seg-set cm-set-repos" role="group" aria-label="Repository">
          {list.map((candidate) => (
            <button key={candidate.id} type="button" aria-pressed={candidate.id === repo?.id} onClick={() => setChosen(candidate.id)}>{candidate.name}</button>
          ))}
        </div>
      )}
      {repo && <RepoSettings key={repo.id} repo={repo} saved={() => void repos.refresh()} />}
    </section>
  );
}

export default definePlugin({ slot: 'settings.manage@1', component: local(ManageSettings) });
