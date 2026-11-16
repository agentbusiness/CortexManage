/**
 * An accepted charter. Left: the rules by level, each with its id, its text, how it is kept and
 * why. Right: the proposal panel. A change to a constraint or a convention becomes a request;
 * a law is edited by the owner alone, who types the law's id to confirm.
 */

import { Menu, useClient } from '@cortex/sdk';
import { useEffect, useState } from 'react';
import { Kept, Seg, shownId, useAct } from './parts';
import { HEAD_LIMIT, LEVELS, STANDARD } from './types';
import type { Charter, Level, Printed, Proposal, Rule } from './types';

const LEVEL_OPTIONS = LEVELS.map(({ value, label }) => ({ value, label }));
/** The panel's choice for a rule that does not exist yet. */
const NEW = '';

function RuleRow({ rule, repo, picked, onPick }: { rule: Rule; repo: string; picked: boolean; onPick?: () => void }) {
  const fromStandard = rule.scope === STANDARD && repo !== STANDARD;
  const unverified = rule.level === 'law' && (rule.kept_by ?? 'text') === 'text';
  const why = [fromStandard ? 'Standard' : '', rule.why || (unverified ? 'A promise nothing verifies yet' : '')].filter(Boolean).join(' · ');
  const classes = ['cx-ch-rule', rule.level === 'law' ? 'law' : '', picked ? 'picked' : '', onPick ? 'pick' : ''].filter(Boolean).join(' ');
  const body = (
    <>
      <span className="cx-ch-id">{shownId(rule, repo)}</span>
      <span className="cx-ch-tx">{rule.text}</span>
      <Kept rule={rule} />
      {why && <span className="cx-ch-wy">{why}</span>}
    </>
  );
  return onPick
    ? <button type="button" className={classes} aria-pressed={picked} onClick={onPick}>{body}</button>
    : <div className={classes}>{body}</div>;
}

/** The request's text: the amendment in words, with the digest the approval is bound to. */
function requestText(proposal: Proposal): string {
  const what = proposal.id
    ? proposal.new ? `change ${proposal.level} ${proposal.id}` : `retire ${proposal.level} ${proposal.id}`
    : `add a ${proposal.level}`;
  return [
    `Charter amendment for ${proposal.repo}: ${what}.`,
    proposal.old ? `Old text: ${proposal.old}` : '',
    proposal.new ? `New text: ${proposal.new}` : 'New text: none, the rule is retired.',
    proposal.why ? `Why: ${proposal.why}` : '',
    `Needs: a ${proposal.needs} change.`,
    proposal.conflicts?.length ? `May conflict with: ${proposal.conflicts.join(', ')}.` : '',
    `Proposal digest: sha256:${proposal.digest}`,
  ].filter(Boolean).join('\n');
}

function requestTitle(proposal: Proposal): string {
  if (!proposal.id) return `Charter: new ${proposal.level}`;
  return proposal.new ? `Charter: change ${proposal.id}` : `Charter: retire ${proposal.id}`;
}

/** What a proposal would do, old and new side by side, and the one way forward for its level. */
function ProposalBlock({ charter, proposal, onDone, onLeave }: { charter: Charter; proposal: Proposal; onDone: (said: string) => void; onLeave: () => void }) {
  const client = useClient();
  const act = useAct();
  const [typed, setTyped] = useState('');
  const isLaw = proposal.needs === 'law';
  // A new law gets the next number; the service says the right id if this guess is behind.
  const laws = charter.rules.filter((rule) => rule.level === 'law').map((rule) => Number(rule.id.slice(1)) || 0);
  const lawId = proposal.id || `L${Math.max(0, ...laws) + 1}`;

  const openRequest = () => act.run(async () => {
    await client.call('manage.request.create', { repo: charter.repo, text: requestText(proposal), title: requestTitle(proposal) });
    onDone('The request is open. The change waits for it like any other request.');
  });
  const editLaw = () => act.run(async () => {
    const edit: Record<string, unknown> = { repo: charter.repo, id: proposal.id, text: proposal.new, confirm: typed.trim() };
    if (!proposal.id) edit.level = 'law';
    if (proposal.why) edit.why = proposal.why;
    await client.call('charter.rule.edit', edit);
    onDone(proposal.new ? `Law ${lawId} is changed.` : `Law ${lawId} is retired.`);
  });

  return (
    <div className={isLaw ? 'cx-ch-block live' : 'cx-ch-block'}>
      <div className="cx-ch-hd">
        <span className={isLaw ? 'pip' : 'pip hollow'} />
        <span className={isLaw ? 'ap live' : 'ap'}>
          {proposal.id ? (proposal.new ? `This changes a ${proposal.level}` : `This retires a ${proposal.level}`) : `This adds a ${proposal.level}`}
        </span>
        {proposal.id && <span className="ap faint">{proposal.id}</span>}
      </div>
      <div className="cx-ch-sides">
        <div>
          <span className="ap faint">Old</span>
          <p className={proposal.old ? 'cx-ch-old' : 'cx-ch-none'}>{proposal.old || 'No rule yet.'}</p>
        </div>
        <div>
          <span className="ap faint">New</span>
          <p className={proposal.new ? 'cx-ch-new' : 'cx-ch-none'}>{proposal.new || 'The rule is retired.'}</p>
        </div>
      </div>
      {proposal.why && <p className="cx-ch-note">Why: {proposal.why}</p>}
      <p className="cx-ch-note">
        {isLaw
          ? 'A law is not changed by a request. Only you can edit it.'
          : `Needs a ${proposal.needs} request: ${proposal.needs === 'major' ? 'your yes on the plan, then on the landing' : 'it is held, then it goes'}.`}
        {' '}
        {proposal.conflicts?.length ? `May conflict with ${proposal.conflicts.join(', ')}: these rules name some of the same paths.` : 'No other rule names the same paths.'}
      </p>
      {act.error && <p className="inline-error">{act.error}</p>}
      {isLaw ? (
        <>
          <label className="cx-ch-label">
            <span className="ap faint">Type {lawId} to confirm</span>
            <span className="field"><input type="text" value={typed} spellCheck={false} autoComplete="off" onChange={(event) => setTyped(event.target.value)} /></span>
          </label>
          <div className="cx-ch-acts">
            <button type="button" className="detent warn" disabled={act.busy || !typed.trim() || (proposal.id !== '' && typed.trim() !== proposal.id)} onClick={() => void editLaw()}>
              {proposal.id ? `Edit law ${proposal.id}` : 'Add law'}
            </button>
            <button type="button" className="detent" onClick={onLeave}>Leave it</button>
          </div>
        </>
      ) : (
        <div className="cx-ch-acts">
          <button type="button" className="detent send" disabled={act.busy} onClick={() => void openRequest()}>Open request</button>
          <button type="button" className="detent" onClick={onLeave}>Leave it</button>
        </div>
      )}
    </div>
  );
}

function ProposalPanel({ charter, target, onTarget }: { charter: Charter; target: string; onTarget: (id: string) => void }) {
  const client = useClient();
  const act = useAct();
  const rule = charter.rules.find((candidate) => candidate.id === target);
  const [level, setLevel] = useState<Level>('convention');
  const [text, setText] = useState('');
  const [why, setWhy] = useState('');
  const [retire, setRetire] = useState(false);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [said, setSaid] = useState('');

  // Another rule picked: its text is where the new wording starts.
  useEffect(() => {
    setText(rule?.text ?? '');
    setWhy('');
    setRetire(false);
    setProposal(null);
    act.clear();
  }, [charter.repo, target]);

  const propose = () => act.run(async () => {
    setSaid('');
    const made = await client.call<Proposal>('charter.propose', { repo: charter.repo, id: target, level: rule ? '' : level, text: retire ? '' : text, why });
    setProposal(made);
  });
  const done = (message: string) => {
    setProposal(null);
    setSaid(message);
    // Back to an empty new rule (the effect above only runs when the picked rule changes).
    setText('');
    setWhy('');
    setRetire(false);
    onTarget(NEW);
  };

  const items = [
    { key: 'new', label: 'New rule', selected: !rule, onSelect: () => onTarget(NEW) },
    ...charter.rules.map((candidate) => ({
      key: candidate.id,
      label: candidate.id,
      note: candidate.text.length > 90 ? `${candidate.text.slice(0, 90)}…` : candidate.text,
      selected: candidate.id === target,
      onSelect: () => onTarget(candidate.id),
    })),
  ];
  const unchanged = rule ? !retire && text.trim() === rule.text : !text.trim();

  return (
    <>
      <div className="cx-ch-block">
        <div className="cx-ch-hd"><span className="ap">Propose a change</span></div>
        <div className="cx-ch-form">
          <div className="cx-ch-line">
            <Menu label="Rule" value={rule ? rule.id : 'New rule'} items={items} />
            {!rule && <Seg label="Level of the new rule" value={level} options={LEVEL_OPTIONS} onChange={(next) => { setLevel(next); setProposal(null); }} />}
            {rule && (
              <>
                <span className="grow" />
                <span className="ap faint">Retire it</span>
                <button type="button" className="sw" aria-pressed={retire} aria-label={`Retire ${rule.id}`} onClick={() => { setRetire((was) => !was); setProposal(null); }} />
              </>
            )}
          </div>
          {!retire && (
            <label className="cx-ch-label">
              <span className="ap faint">New text: one or two plain sentences</span>
              <textarea className="cx-ch-area" rows={4} value={text} placeholder="What is true, then what to do instead" onChange={(event) => { setText(event.target.value); setProposal(null); }} />
            </label>
          )}
          <label className="cx-ch-label">
            <span className="ap faint">Why</span>
            <textarea className="cx-ch-area" rows={2} value={why} onChange={(event) => { setWhy(event.target.value); setProposal(null); }} />
          </label>
          {act.error && <p className="inline-error">{act.error}</p>}
          <div className="cx-ch-acts">
            <button type="button" className="detent" disabled={act.busy || unchanged} onClick={() => void propose()}>Propose</button>
          </div>
        </div>
      </div>
      {proposal && <ProposalBlock charter={charter} proposal={proposal} onDone={done} onLeave={() => setProposal(null)} />}
      {said && <p className="cx-ch-note counter">{said}</p>}
    </>
  );
}

export function AcceptedView({ charter, printed }: { charter: Charter; printed: Printed | undefined }) {
  const [target, setTarget] = useState(NEW);
  // The picked rule can leave the charter (retired, or another repository is shown).
  useEffect(() => {
    if (target && !charter.rules.some((rule) => rule.id === target)) setTarget(NEW);
  }, [charter, target]);
  const identity = charter.identity ?? {};
  const over = printed !== undefined && printed.head_chars > HEAD_LIMIT;

  return (
    <div className="cx-ch-split">
      <div className="cx-ch-left">
        {LEVELS.map((level) => {
          const shared = (charter.standard ?? []).filter((rule) => rule.level === level.value);
          const own = charter.rules.filter((rule) => rule.level === level.value);
          return (
            <section key={level.value}>
              <div className="cx-ch-lvl">
                <span className={level.value === 'law' ? 'ap live' : level.value === 'constraint' ? 'ap origin' : 'ap'}>{level.plural}</span>
                <span className="ap faint">{level.changedBy}</span>
              </div>
              {shared.length + own.length === 0 && <p className="cx-ch-note">None.</p>}
              {shared.map((rule) => <RuleRow key={`s-${rule.id}`} rule={rule} repo={charter.repo} picked={false} />)}
              {own.map((rule) => <RuleRow key={rule.id} rule={rule} repo={charter.repo} picked={rule.id === target} onPick={() => setTarget(rule.id)} />)}
            </section>
          );
        })}
        {(identity.what || identity.who_for || identity.never?.length) && (
          <section>
            <div className="cx-ch-lvl"><span className="ap">What it is</span></div>
            {identity.what && <p className="cx-ch-ident">{identity.what}</p>}
            {identity.who_for && <p className="cx-ch-ident"><span className="ap faint">For</span> {identity.who_for}</p>}
            {identity.never?.map((line) => <p key={line} className="cx-ch-ident"><span className="ap faint">Never</span> {line}</p>)}
          </section>
        )}
      </div>
      <div className="vrule" />
      <div className="cx-ch-right">
        <ProposalPanel charter={charter} target={target} onTarget={setTarget} />
        {printed && (
          <p className={over ? 'cx-ch-note live' : 'cx-ch-note'}>
            In the printed AGENTS.md the laws and constraints end at character {printed.head_chars} of the {HEAD_LIMIT} that CortexCode&rsquo;s model reads.
            {over && ' They do not fit: the model will not see every one.'}
          </p>
        )}
      </div>
    </div>
  );
}
