/**
 * pane.watch@1 — the Watch tab, for the open request's package while its job runs: the strip of
 * facts (kept live from the `manage.watch` topic), the rules watching it and which fired, what
 * happens at the next failure, the digest, and every time CortexManage stepped in.
 */

import { useState } from 'react';
import { definePlugin, useEvent, useNow } from '@cortex/sdk';
import { WATCH, WORD_LABEL, count, loadFailure, local, pipClass, span, timeOf, useOpen, type PackageView, type Strip, type WatchBeat } from './manage';
import './manage.css';
import './watch.css';

/** The watch's rules in a few words (docs/specs/01-WORK.md). */
const RULE_TEXT: Record<string, string> = {
  W1: 'same command fails three times',
  W2: 'same call, same result four times',
  W3: 'reading without writing',
  W4: "outside the task's files",
  W5: 'standard files',
  W6: 'charter laws',
  W7: 'budget',
  W8: 'missed beats',
};

const ACTION_LABEL: Record<string, string> = {
  steer: 'Steer',
  restart: 'Restart',
  stop: 'Stop',
  refuse: 'Refused',
  note: 'Note',
};

/** What the next failure does: steer once, restart once, then the person. */
function nextRung(pack: PackageView, strip: Strip | undefined): string {
  if (pack.word === 'needs_you' || pack.word === 'stuck') return 'The ladder is used up: this waits on you, with the evidence.';
  if (!strip) return 'No job is running for this package, so nothing is being watched.';
  if (strip.restarted > 0) return 'Steer once, restart once, then you. Both are used. The next failure brings this to Waits on you with the evidence.';
  if (strip.steered > 0) return 'Steer once, restart once, then you. The steer is used. If a rule fires again, the task starts again from the last checkpoint.';
  return 'Steer once, restart once, then you. Nothing is used yet: if a rule fires, the builder is told once what to do instead.';
}

function StripView({ strip, now }: { strip: Strip; now: number }) {
  return (
    <div className="cm-strip">
      <div><span className="ap faint">Task</span><span className="num">{strip.task} of {strip.tasks} · attempt {strip.attempt}</span></div>
      <div><span className="ap faint">Last checkpoint</span><span className="num">{strip.checkpoint_ms ? `${span(Math.max(0, now - strip.checkpoint_ms))} ago` : 'none yet'}</span></div>
      <div><span className="ap faint">Changed</span><span className="num">{count(strip.files, 'file')} · {strip.lines}{strip.limit_lines ? ` of ${strip.limit_lines}` : ''} lines</span></div>
      <div><span className="ap faint">Scope</span><span className={`num${strip.in_scope ? '' : ' bad'}`}>{strip.in_scope ? 'in scope' : 'out of scope'}</span></div>
      <div><span className="ap faint">Time</span><span className="num">{strip.minutes}{strip.limit_minutes ? ` of ${strip.limit_minutes}` : ''} min</span></div>
      <div><span className="ap faint">Steered · restarted</span><span className={`num${strip.steered + strip.restarted > 0 ? ' bad' : ''}`}>{strip.steered} · {strip.restarted}</span></div>
      {strip.now && <div className="wide"><span className="ap faint">Now</span><span className="now">{strip.now}</span></div>}
    </div>
  );
}

function Watch({ active }: { active?: boolean }) {
  const { selection, request, pack, packageId } = useOpen(active !== false);
  // The newest beat of this package; the package's own answer until one arrives.
  const [beat, setBeat] = useState<{ beat: WatchBeat; at: number } | null>(null);
  useEvent(packageId ? WATCH : null, (_topic, data) => {
    const next = data as WatchBeat | null;
    if (next && next.package === packageId && next.strip) setBeat({ beat: next, at: Date.now() });
  });
  const now = useNow(30_000, active !== false && Boolean(packageId));
  if (selection.view !== 'session' || !selection.request) {
    return <div className="cm-pane" data-build={__CORTEX_BUILD__}><p className="cm-line flush">Open a request in the sidebar to watch its job.</p></div>;
  }
  const data = pack.data;
  // A beat counts while it is newer than the package's own answer. Once the job is over the
  // package has no strip, and the last beat is history.
  const live = beat && beat.beat.package === packageId && beat.at > pack.at ? beat.beat : null;
  const strip = data?.strip ? live?.strip ?? data.strip : undefined;
  const digest = (data?.strip ? live?.digest : '') || data?.digest || '';
  const steps = [...(data?.interventions ?? [])].sort((a, b) => b.ms - a.ms);
  return (
    <div className="cm-pane" data-build={__CORTEX_BUILD__} data-package={packageId ?? ''}>
      <div className="cm-pane-h">
        {data && <span className={pipClass(data.word)} />}
        <span className="ap">{data ? WORD_LABEL[data.word] ?? data.word : 'Watch'}</span>
        <span className="grow" />
        <span className="ap faint">{packageId ?? selection.request}</span>
      </div>
      {request.error && !request.data && <p className="inline-error">{loadFailure('The request', request)}</p>}
      {pack.error && !data && <p className="inline-error">{loadFailure('The package', pack)}</p>}
      {request.data && !packageId && <p className="cm-line flush">This request is not in a package yet, so no job runs for it.</p>}
      {data && (
        <>
          <div className="cm-sec">
            <span className="ap">On track?</span>
            {strip ? <StripView strip={strip} now={now} /> : <p className="cm-note faint">No job is running for this package.</p>}
          </div>
          {strip && strip.rules.length > 0 && (
            <div className="cm-sec">
              <span className="ap">Rules watching</span>
              {strip.rules.map(([rule, fired]) => (
                <div key={rule} className="cm-row" data-rule={rule} data-fired={fired ? 'true' : 'false'}>
                  <span>{RULE_TEXT[rule] ? `${rule} ${RULE_TEXT[rule]}` : rule}</span>
                  <span className={`ap ${fired ? 'live' : 'faint'}`}>{fired ? 'Fired' : 'Quiet'}</span>
                </div>
              ))}
            </div>
          )}
          <div className="cm-sec">
            <span className="ap">Next rung</span>
            <p className="cm-note">{nextRung(data, strip)}</p>
          </div>
          {digest && (
            <div className="cm-sec">
              <span className="ap">Digest</span>
              <p className="cm-note">{digest}</p>
            </div>
          )}
          <div className="cm-sec">
            <span className="ap">Stepped in</span>
            {steps.length === 0 && <p className="cm-note faint">CortexManage has not stepped in.</p>}
            {steps.map((step, index) => (
              <div key={`${step.ms}:${index}`} className="cm-step" data-rule={step.rule}>
                <div className="cm-step-h">
                  <span className={`ap ${step.action === 'note' ? 'faint' : 'live'}`}>
                    {timeOf(step.ms, now)} · {ACTION_LABEL[step.action] ?? step.action} · {RULE_TEXT[step.rule] ?? step.rule}
                  </span>
                  {step.task > 0 && <span className="ap faint">task {step.task}</span>}
                </div>
                <p>{step.fact}</p>
                {step.text && <p className="said">{step.text}</p>}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

export default definePlugin<{ active?: boolean }>({ slot: 'pane.watch@1', component: local(Watch) });
