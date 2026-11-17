/**
 * A draft: every rule with its level set by a segmented control, its text and how it is kept
 * edited in place; the identity; and Accept, which makes it version 1.0.
 */

import { useClient } from '@cortex/sdk';
import { useEffect, useState } from 'react';
import { ImportPanel } from './Import';
import { Kept, Seg, lines, useAct } from './parts';
import { HEAD_LIMIT, KEPT, LEVELS } from './types';
import type { Charter, KeptBy, Level, Printed, Rule } from './types';

const LEVEL_OPTIONS = LEVELS.map(({ value, label }) => ({ value, label }));

/** One rule of the draft. Its level is set at once; the rest is edited when opened. */
function DraftRule({ repo, rule }: { repo: string; rule: Rule }) {
  const client = useClient();
  const act = useAct();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState(rule.text);
  const [why, setWhy] = useState(rule.why ?? '');
  const [paths, setPaths] = useState((rule.applies_to ?? []).join(', '));
  const [kept, setKept] = useState<KeptBy>(rule.kept_by || 'text');
  const [check, setCheck] = useState(rule.check ?? '');

  const setLevel = (level: Level) => act.run(() => client.call('charter.rule.edit', { repo, id: rule.id, level }));
  const save = () => act.run(async () => {
    await client.call('charter.rule.edit', {
      repo,
      id: rule.id,
      text,
      why,
      applies_to: paths.split(',').map((glob) => glob.trim()).filter(Boolean),
      kept_by: kept,
      check: kept === 'check' ? check : '',
    });
    setOpen(false);
  });
  const remove = () => act.run(() => client.call('charter.rule.delete', { repo, id: rule.id }));

  return (
    <div className={rule.level === 'law' ? 'cx-ch-rule law draft' : 'cx-ch-rule draft'}>
      <span className="cx-ch-id">{rule.id}</span>
      <span className="cx-ch-tx">{rule.text}</span>
      <Seg label={`Level of ${rule.id}`} value={rule.level} options={LEVEL_OPTIONS} disabled={act.busy} onChange={(level) => void setLevel(level)} />
      <div className="cx-ch-under">
        <Kept rule={rule} />
        <span className="grow" />
        <button type="button" className="detent tiny" aria-expanded={open} onClick={() => setOpen((was) => !was)}>{open ? 'Close' : 'Edit'}</button>
        <button type="button" className="detent tiny" disabled={act.busy} onClick={() => void remove()}>Remove</button>
      </div>
      {act.error && <p className="inline-error cx-ch-wide">{act.error}</p>}
      {open && (
        <div className="cx-ch-form cx-ch-wide">
          <label className="cx-ch-label">
            <span className="ap faint">Text: one or two plain sentences</span>
            <textarea className="cx-ch-area" rows={3} value={text} onChange={(event) => setText(event.target.value)} />
          </label>
          <label className="cx-ch-label">
            <span className="ap faint">Why: one sentence, for people</span>
            <textarea className="cx-ch-area" rows={2} value={why} onChange={(event) => setWhy(event.target.value)} />
          </label>
          <label className="cx-ch-label">
            <span className="ap faint">Applies to: path globs, comma separated; empty is everywhere</span>
            <span className="field"><input type="text" value={paths} spellCheck={false} placeholder="plugins/*/ui/**" onChange={(event) => setPaths(event.target.value)} /></span>
          </label>
          <div className="cx-ch-line">
            <span className="ap faint">Kept by</span>
            <Seg label={`How ${rule.id} is kept`} value={kept} options={KEPT} onChange={setKept} />
            {kept === 'check' && <span className="field cx-ch-grow"><input type="text" value={check} spellCheck={false} placeholder="The check's name" onChange={(event) => setCheck(event.target.value)} /></span>}
          </div>
          <div className="cx-ch-acts">
            <button type="button" className="detent" disabled={act.busy || !text.trim()} onClick={() => void save()}>Save</button>
          </div>
        </div>
      )}
    </div>
  );
}

/** What the app is: one paragraph, who it is for, what it must never become. */
function IdentityForm({ charter }: { charter: Charter }) {
  const client = useClient();
  const act = useAct();
  const [what, setWhat] = useState(charter.identity?.what ?? '');
  const [whoFor, setWhoFor] = useState(charter.identity?.who_for ?? '');
  const [never, setNever] = useState((charter.identity?.never ?? []).join('\n'));
  // Another import or another repository brings another identity.
  useEffect(() => {
    setWhat(charter.identity?.what ?? '');
    setWhoFor(charter.identity?.who_for ?? '');
    setNever((charter.identity?.never ?? []).join('\n'));
  }, [charter.repo, charter.identity?.what, charter.identity?.who_for, (charter.identity?.never ?? []).join('\n')]);

  const save = () => act.run(() => client.call('charter.rule.edit', { repo: charter.repo, identity: { what, who_for: whoFor, never: lines(never) } }));

  return (
    <div className="cx-ch-form">
      <label className="cx-ch-label">
        <span className="ap faint">What it is</span>
        <textarea className="cx-ch-area" rows={3} value={what} onChange={(event) => setWhat(event.target.value)} />
      </label>
      <label className="cx-ch-label">
        <span className="ap faint">Who it is for</span>
        <textarea className="cx-ch-area" rows={1} value={whoFor} onChange={(event) => setWhoFor(event.target.value)} />
      </label>
      <label className="cx-ch-label">
        <span className="ap faint">What it must never become: one line each</span>
        <textarea className="cx-ch-area" rows={3} value={never} onChange={(event) => setNever(event.target.value)} />
      </label>
      {act.error && <p className="inline-error">{act.error}</p>}
      <div className="cx-ch-acts">
        <button type="button" className="detent" disabled={act.busy} onClick={() => void save()}>Save</button>
      </div>
    </div>
  );
}

function AddRule({ repo }: { repo: string }) {
  const client = useClient();
  const act = useAct();
  const [text, setText] = useState('');
  const [level, setLevel] = useState<Level>('convention');
  const add = () => act.run(async () => {
    await client.call('charter.rule.edit', { repo, level, text });
    setText('');
  });
  return (
    <div className="cx-ch-form">
      <textarea className="cx-ch-area" rows={3} value={text} placeholder="What is true, then what to do instead" onChange={(event) => setText(event.target.value)} />
      {act.error && <p className="inline-error">{act.error}</p>}
      <div className="cx-ch-line">
        <Seg label="Level of the new rule" value={level} options={LEVEL_OPTIONS} onChange={setLevel} />
        <span className="grow" />
        <button type="button" className="detent" disabled={act.busy || !text.trim()} onClick={() => void add()}>Add rule</button>
      </div>
    </div>
  );
}

export function DraftView({ charter, printed }: { charter: Charter; printed: Printed | undefined }) {
  const client = useClient();
  const act = useAct();
  const [again, setAgain] = useState(false);
  const accept = () => act.run(() => client.call('charter.accept', { repo: charter.repo }));
  const over = printed !== undefined && printed.head_chars > HEAD_LIMIT;

  return (
    <div className="cx-ch-split">
      <div className="cx-ch-left">
        {LEVELS.map((level) => {
          const rules = charter.rules.filter((rule) => rule.level === level.value);
          return (
            <section key={level.value}>
              <div className="cx-ch-lvl">
                <span className={level.value === 'law' ? 'ap live' : level.value === 'constraint' ? 'ap origin' : 'ap'}>{level.plural}</span>
                <span className="ap faint">{rules.length}</span>
              </div>
              {rules.length === 0 && <p className="cx-ch-note">None yet.</p>}
              {rules.map((rule) => <DraftRule key={rule.id} repo={charter.repo} rule={rule} />)}
            </section>
          );
        })}
      </div>
      <div className="vrule" />
      <div className="cx-ch-right">
        <div className="cx-ch-block">
          <div className="cx-ch-hd"><span className="pip hollow" /><span className="ap">Draft</span><span className="ap faint">not in force yet</span></div>
          {printed && over && <p className="cx-ch-note live">The laws and constraints are too long for the builder to read in full: shorten them, or move some to conventions.</p>}
          {act.error && <p className="inline-error">{act.error}</p>}
          <div className="cx-ch-acts">
            <button type="button" className="detent send" disabled={act.busy || charter.rules.length === 0} onClick={() => void accept()}>Accept draft</button>
          </div>
        </div>
        <div className="cx-ch-block">
          <div className="cx-ch-hd"><span className="ap">New rule</span></div>
          <AddRule repo={charter.repo} />
        </div>
        <div className="cx-ch-block">
          <div className="cx-ch-hd"><span className="ap">What it is</span></div>
          <IdentityForm charter={charter} />
        </div>
        <div className="cx-ch-block">
          <div className="cx-ch-hd">
            <span className="ap">Import again</span>
            <span className="grow" />
            <button type="button" className="detent tiny" aria-expanded={again} onClick={() => setAgain((was) => !was)}>{again ? 'Close' : 'Open'}</button>
          </div>
          {again && <ImportPanel repo={charter.repo} replaces onDone={() => setAgain(false)} />}
        </div>
      </div>
    </div>
  );
}
