/**
 * One block of the thread per kind of journal entry the manage plugin writes
 * (`manage::kinds`), beside the person's own messages and the manager's lines.
 */

import { useState, type ReactNode } from 'react';
import { clock, type JournalEntry } from '@cortex/sdk';
import type { Ask, Finding, Plan, Point, Question, RequestView } from './types';

type Data = JournalEntry['data'];

const str = (value: unknown): string => (typeof value === 'string' ? value : '');
const num = (value: unknown): number | null => (typeof value === 'number' && Number.isFinite(value) ? value : null);
const list = (value: unknown): string[] => (Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string' && item !== '') : []);
const record = (value: unknown): Record<string, unknown> | null => (value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null);
const capital = (text: string): string => (text ? text[0]!.toUpperCase() + text.slice(1).replace(/_/g, ' ') : text);
const count = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** What the thread's blocks may ask of the view: each is one contract call. */
export interface Acts {
  request: RequestView;
  /** A call is on its way: the detents wait. */
  busy: boolean;
  answer(answers: Array<[string, string[]]>): void;
  packageEdit(pkg: string, edit: { start_now?: boolean; stop?: boolean; unbundle?: string }): void;
  decide(ask: Ask, action: string): void;
}

function Block({ pip, label, labelTone, aside, time, tone, children, seq }: {
  pip?: 'done' | 'run' | 'hollow' | 'fail' | 'live';
  label: string;
  labelTone?: 'live' | 'origin';
  aside?: string;
  time?: number;
  tone?: 'counter';
  children?: ReactNode;
  seq: number;
}) {
  return (
    <section className={`block${tone ? ` ${tone}` : ''}`} data-seq={seq}>
      <div className="hd">
        <span className={`pip${pip && pip !== 'live' ? ` ${pip}` : ''}`} aria-hidden />
        <span className={`ap${labelTone ? ` ${labelTone}` : ''}`}>{label}</span>
        {aside && <span className="ap faint">· {aside}</span>}
        <span className="grow" />
        {time !== undefined && <time className="ap faint">{clock(time)}</time>}
      </div>
      {children}
    </section>
  );
}

/** A person's message, or a line of the manager's. */
export function Message({ entry, who, tone }: { entry: JournalEntry; who: string; tone?: 'live' }) {
  const text = str(entry.data.text);
  if (!text) return null;
  return (
    <article className="msg" data-seq={entry.seq}>
      <div className={`who ap ${tone ?? 'faint'}`}>{who} · {clock(entry.ts_ms)}</div>
      {text.split(/\n+/).map((line, index) => <p key={index}>{line}</p>)}
    </article>
  );
}

/** `assessment`: what kind of work it is and how much can go wrong, as keys and values. */
export function Assessment({ entry }: { entry: JournalEntry }) {
  const data = entry.data;
  const tier = str(data.tier);
  const reasons = list(data.reasons);
  const surface = list(data.surface);
  const parts = list(data.parts);
  const urgent = data.urgent === true;
  return (
    <Block seq={entry.seq} pip="done" label="Assessment" time={entry.ts_ms}>
      <div className="kv">
        {str(data.kind) && <><span className="ap faint">Kind</span><span>{capital(str(data.kind))}</span></>}
        {tier && (
          <>
            <span className="ap faint">Impact</span>
            <span><span className={`tag ${tier}`}>{tier}</span> {reasons.join(' ')}</span>
          </>
        )}
        {str(data.tier_floor) && str(data.tier_floor) !== tier && <><span className="ap faint">At least</span><span>{capital(str(data.tier_floor))}, from the files and facts.</span></>}
        <span className="ap faint">Urgent</span>
        {urgent
          ? <span>Yes. <q>{str(data.urgent_because)}</q></span>
          : <span className="sub">No.</span>}
        {str(data.because) && <><span className="ap faint">Because</span><span><q>{str(data.because)}</q></span></>}
        {surface.length > 0 && <><span className="ap faint">Reaches</span><span>{surface.map(capital).join(', ')}</span></>}
        {parts.length > 0 && <><span className="ap faint">{parts.length === 1 ? 'Part' : 'Parts'}</span><span>{parts.map((part, index) => <span key={part}>{index > 0 && ', '}<code>{part}</code></span>)}</span></>}
      </div>
    </Block>
  );
}

/** `clarify`: the questions as a form while they are open, as questions and answers afterwards. */
export function Clarify({ entry, open, acts }: { entry: JournalEntry; open: boolean; acts: Acts }) {
  const asked = (Array.isArray(entry.data.questions) ? entry.data.questions : []) as unknown as Question[];
  // The answers given so far live on the request; the entry holds the questions as they were asked.
  const current = new Map((acts.request.questions ?? []).map((question) => [question.id, question]));
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [written, setWritten] = useState<Record<string, string>>({});
  if (asked.length === 0) return null;

  const pick = (question: Question, option: string) => setPicked((now) => {
    const chosen = now[question.id] ?? [];
    const next = question.multi
      ? (chosen.includes(option) ? chosen.filter((other) => other !== option) : [...chosen, option])
      : (chosen.includes(option) ? [] : [option]);
    return { ...now, [question.id]: next };
  });
  const answers = (): Array<[string, string[]]> => asked
    .map((question): [string, string[]] => {
      const own = (written[question.id] ?? '').trim();
      return [question.id, [...(picked[question.id] ?? []), ...(own ? [own] : [])]];
    })
    .filter(([, given]) => given.length > 0);
  const ready = open && answers().length > 0 && !acts.busy;

  return (
    <Block seq={entry.seq} pip={open ? undefined : 'done'} label={open ? 'Before this can be planned' : 'Questions'} labelTone={open ? 'live' : undefined} time={entry.ts_ms} tone={open ? 'counter' : undefined}>
      {asked.map((question) => {
        const options = question.options ?? [];
        const given = current.get(question.id)?.answer ?? question.answer ?? [];
        const chosen = picked[question.id] ?? [];
        return (
          <div className="question" key={question.id}>
            <p>{question.text}</p>
            {!open && <p className="sub">{given.length > 0 ? given.join(', ') : 'Not answered.'}</p>}
            {open && options.length > 0 && (question.multi
              ? (
                <div className="opts" role="group" aria-label={question.text}>
                  {options.map((option) => (
                    <button key={option} type="button" className="detent" aria-pressed={chosen.includes(option)} onClick={() => pick(question, option)}>{option}</button>
                  ))}
                </div>
              )
              : (
                <div className="seg-set" role="group" aria-label={question.text}>
                  {options.map((option) => (
                    <button key={option} type="button" aria-pressed={chosen.includes(option)} onClick={() => pick(question, option)}>{option}</button>
                  ))}
                </div>
              ))}
            {open && (
              <input
                className="write one"
                type="text"
                value={written[question.id] ?? ''}
                placeholder={options.length > 0 ? 'Or in your own words…' : 'Your answer…'}
                aria-label={`${question.text} (your own words)`}
                onChange={(event) => setWritten((now) => ({ ...now, [question.id]: event.target.value }))}
              />
            )}
          </div>
        );
      })}
      {open && (
        <div className="acts">
          <button type="button" className="detent send" disabled={!ready} aria-pressed={ready} onClick={() => acts.answer(answers())}>Send</button>
        </div>
      )}
    </Block>
  );
}

function Task({ point, checks }: { point: Point; checks: boolean }) {
  const verdict = point.verdict ?? '';
  const walks = checks ? point.walks ?? [] : [];
  return (
    <div className={`task${verdict === 'cut' ? ' cut' : ''}`}>
      <span className="kk">{point.n}</span>
      <span className="tt">{point.title}</span>
      <span className={`v ${verdict}`}>{verdict ? capital(verdict) : ''}</span>
      {walks.map((walk) => (
        <span className="ac" key={walk.id}>
          <b>Walk</b>{walk.do} {walk.see}
          {walk.method === 'command' && walk.command && <> <code>{walk.command}</code></>}
        </span>
      ))}
      {verdict === 'cut' && point.verdict_reason && <span className="ac">Left out: {point.verdict_reason}</span>}
    </div>
  );
}

/** `manage_plan`: the tasks with their walks and the validator's mark; cut ones struck through. */
export function PlanBlock({ entry }: { entry: JournalEntry }) {
  const [checks, setChecks] = useState(false);
  const plan = record(entry.data.plan) as unknown as Plan | null;
  if (!plan || !Array.isArray(plan.points)) return null;
  const cut = plan.points.filter((point) => point.verdict === 'cut').length;
  const waiting = plan.points.filter((point) => point.verdict === 'ask').length;
  const aside = [count(plan.points.length, 'point', 'points'), cut > 0 ? `validator cut ${cut}` : '', waiting > 0 ? `${waiting} for you to decide` : ''].filter(Boolean).join(' · ');
  const walks = plan.points.reduce((sum, point) => sum + (point.walks ?? []).length, 0);
  return (
    <Block seq={entry.seq} pip="done" label={`Plan v${plan.version}`} aside={aside} time={entry.ts_ms}>
      {plan.goal && <p className="lead">{plan.goal}</p>}
      {plan.points.map((point) => <Task key={point.n} point={point} checks={checks} />)}
      {/* How each point will be proven is one press away, not in the way. */}
      {walks > 0 && (
        <p className="foot">
          <button type="button" className="detent tiny" aria-expanded={checks} onClick={() => setChecks(!checks)}>{checks ? 'Hide the checks' : `How it is checked (${walks})`}</button>
        </p>
      )}
      {checks && plan.architecture && <p className="foot sub">{plan.architecture}</p>}
      {checks && plan.note && <p className="foot sub">{plan.note}</p>}
    </Block>
  );
}

/** `package`: held, joined with others, or started; while it waits, the hold's three buttons. */
export function PackageBlock({ entry, open, acts }: { entry: JournalEntry; open: boolean; acts: Acts }) {
  const data = entry.data;
  const pkg = str(data.package);
  const joined = list(data.joined);
  const until = num(data.hold_until_ms);
  const label = joined.length > 0 ? 'Joined a package' : data.urgent === true ? 'Urgent' : until ? 'Held' : 'Package';
  return (
    <Block seq={entry.seq} pip="hollow" label={label} labelTone="origin" time={entry.ts_ms} tone="counter">
      {open && until !== null && until > 0
        ? <p className="body">Builds at {clock(until)}{joined.length > 0 ? `, together with ${joined.join(', ')}` : ''}.</p>
        : str(data.text) && <p className="body">{str(data.text)}</p>}
      {open && pkg && (
        <div className="acts">
          <button type="button" className="detent" disabled={acts.busy} onClick={() => acts.packageEdit(pkg, { start_now: true })}>Start now</button>
          <button type="button" className="detent" disabled={acts.busy} onClick={() => acts.packageEdit(pkg, { stop: true })}>Stop it</button>
          {joined.length > 0 && <button type="button" className="detent" disabled={acts.busy} onClick={() => acts.packageEdit(pkg, { unbundle: acts.request.id })}>Build apart</button>}
        </div>
      )}
    </Block>
  );
}

/** `watch`: a checkpoint with its drift answer, or one time the manager stepped in, with the rule. */
export function WatchBlock({ entry }: { entry: JournalEntry }) {
  const data = entry.data;
  const checkpoint = record(data.checkpoint);
  const drift = record(data.drift);
  if (checkpoint || drift) {
    const task = num(checkpoint?.task) ?? num(drift?.task);
    const files = num(checkpoint?.files);
    const lines = num(checkpoint?.lines);
    const aside = [task !== null ? `task ${task}` : '', files !== null ? count(files, 'file', 'files') : '', lines !== null ? count(lines, 'line', 'lines') : ''].filter(Boolean).join(' · ');
    const yes = drift?.does_it === true;
    const where = str(drift?.file) ? `${str(drift?.file)}${num(drift?.line) ? `:${num(drift?.line)}` : ''}` : '';
    return (
      <Block seq={entry.seq} pip="done" label="Checkpoint" aside={aside} time={entry.ts_ms}>
        {drift && (
          <p className="body">
            Does the change do what {task !== null ? `task ${task}` : 'the task'} says? <b>{yes ? 'Yes' : 'No'}</b>
            {yes && where && <>, <code>{where}</code></>}. {str(drift.reason)}
          </p>
        )}
      </Block>
    );
  }
  const action = str(data.action);
  const rule = str(data.rule);
  const task = num(data.task);
  // A note changes nothing for the builder; a steer, restart, stop or refusal does.
  const stepped = action !== '' && action !== 'note';
  return (
    <Block seq={entry.seq} pip={stepped ? 'live' : 'hollow'} label={[capital(action || 'note'), rule ? `rule ${rule}` : ''].filter(Boolean).join(' · ')} labelTone={stepped ? 'live' : undefined} aside={task ? `task ${task}` : undefined} time={entry.ts_ms}>
      {str(data.fact) && <p className="body">{str(data.fact)}</p>}
      {str(data.text) && <p className="body"><q>{str(data.text)}</q></p>}
    </Block>
  );
}

/** `gate`: one gate's answer and what it found. Never shown as passed unless it says so. */
export function GateBlock({ entry }: { entry: JournalEntry }) {
  const data = entry.data;
  const result = str(data.result);
  const findings = (Array.isArray(data.findings) ? data.findings : []) as unknown as Finding[];
  const attempt = num(data.attempt);
  const pip = result === 'passed' ? 'done' : result === 'failed' ? 'fail' : 'hollow';
  return (
    <Block seq={entry.seq} pip={pip} label={`Gate · ${capital(str(data.kind))}`} labelTone={result === 'failed' ? 'live' : undefined} aside={[capital(result || 'not run'), attempt ? `attempt ${attempt}` : ''].filter(Boolean).join(' · ')} time={entry.ts_ms}>
      {findings.map((finding, index) => (
        <p className="body" key={index}>{finding.title}{finding.evidence && <> <span className="sub">{finding.evidence}</span></>}</p>
      ))}
    </Block>
  );
}

/** `stage`: a stage was reached. One line. */
export function StageLine({ entry }: { entry: JournalEntry }) {
  return (
    <div className="line" data-seq={entry.seq}>
      <span className="pip done" aria-hidden />
      <span className="ap">{capital(str(entry.data.stage))}</span>
      <span className="said">{str(entry.data.text)}</span>
      <span className="grow" />
      <time className="ap faint">{clock(entry.ts_ms)}</time>
    </div>
  );
}

const ACTIONS: Record<string, string> = {
  approve: 'Approve',
  send_back: 'Send back',
  decline: 'Decline',
  rerun: 'Run again',
  take_over: 'Take over',
  first: 'The first',
  second: 'The second',
};

/** `ask`: a decision that waits for a person, with its buttons; or the line saying it was answered. */
export function AskBlock({ entry, open, acts }: { entry: JournalEntry; open: boolean; acts: Acts }) {
  const ask = record(entry.data.ask) as unknown as Ask | null;
  if (!ask) {
    const action = str(entry.data.action);
    if (!action) return null;
    return (
      <div className="line" data-seq={entry.seq}>
        <span className="pip done" aria-hidden />
        <span className="ap">{ACTIONS[action] ?? capital(action)}</span>
        <span className="said">{str(entry.data.by) ? `by ${str(entry.data.by)}` : ''}</span>
        <span className="grow" />
        <time className="ap faint">{clock(entry.ts_ms)}</time>
      </div>
    );
  }
  return (
    <Block seq={entry.seq} pip={open ? undefined : 'done'} label={open ? 'Waits on you' : 'Was asked'} labelTone={open ? 'live' : undefined} time={entry.ts_ms} tone={open ? 'counter' : undefined}>
      <p className="lead">{ask.subject || ask.title}</p>
      <div className="kv">
        {ask.why && <><span className="ap faint">Why</span><span>{ask.why}</span></>}
        {ask.if_yes && <><span className="ap faint">If yes</span><span>{ask.if_yes}</span></>}
        {ask.if_no && <><span className="ap faint">If no</span><span>{ask.if_no}</span></>}
      </div>
      {open && (
        <div className="acts">
          {(ask.actions ?? []).map((action) => (
            <button key={action} type="button" className={`detent${action === 'decline' ? ' warn' : ''}`} disabled={acts.busy} onClick={() => acts.decide(ask, action)}>{ACTIONS[action] ?? capital(action)}</button>
          ))}
        </div>
      )}
    </Block>
  );
}

/** A failure written into the thread (a message that could not be passed on). */
export function Trouble({ entry }: { entry: JournalEntry }) {
  return (
    <div className="notice contradiction" role="alert" data-seq={entry.seq}>
      <span className="kindmark" />
      <span className="ap lbl">Trouble</span>
      <p>{str(entry.data.message)}</p>
    </div>
  );
}

/** A plain line of the frame's own (`note`). */
export function Note({ entry }: { entry: JournalEntry }) {
  const text = str((entry.data as Data).text);
  if (!text) return null;
  return <p className="aside sub" data-seq={entry.seq}>{text}</p>;
}
