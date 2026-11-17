/**
 * pane.card@1 — the Card tab, for the request that is open: the stages it has reached and when,
 * its package (requests, tasks, tier, branch, budget), the gates with their results, and where it
 * builds on the mesh.
 */

import { definePlugin, useNow } from '@cortex/sdk';
import {
  GATE_LABEL,
  RESULT_LABEL,
  STAGES,
  STAGE_LABEL,
  TIER_LABEL,
  WORD_LABEL,
  count,
  latestGates,
  loadFailure,
  local,
  pipClass,
  resultPip,
  timeOf,
  useOpen,
  type Gate,
  type PackageView,
  type RequestView,
} from './manage';
import './manage.css';
import './card.css';

/** The gates a package always passes; a person's yes is shown only when one was asked for. */
const ALWAYS = ['scope', 'charter', 'build', 'walks', 'converge'];

function Ladder({ request, pack, now }: { request: RequestView; pack: PackageView | undefined; now: number }) {
  const reached = new Map<string, number>();
  for (const [stage, ms] of request.stages ?? []) reached.set(stage, ms);
  const current = STAGES.indexOf(request.stage as (typeof STAGES)[number]);
  return (
    <ul className="cm-ladder">
      {STAGES.map((stage, index) => {
        const isNow = stage === request.stage && !request.ended_as;
        // After a rework a later stage was reached once already; only what lies behind counts.
        const passed = reached.has(stage) && !isNow && (current < 0 || index < current || Boolean(request.ended_as));
        let when = passed || isNow ? timeOf(reached.get(stage) ?? 0, now) : '';
        // What ends by the clock says when: the hold and the soak.
        if (isNow && stage === 'held' && pack) when = pack.stopped ? 'stopped' : pack.hold_until_ms ? `until ${timeOf(pack.hold_until_ms, now)}` : 'until your yes';
        if (isNow && stage === 'soaking' && pack?.soak_until_ms) when = `until ${timeOf(pack.soak_until_ms, now)}`;
        return (
          <li key={stage} className={isNow ? 'now' : passed ? 'd' : undefined} data-stage={stage}>
            <span className={isNow ? pipClass(request.word) : passed ? 'cx-pip cx-pip-done' : 'cx-pip'} />
            {STAGE_LABEL[stage]}
            <span className="w">{when}</span>
          </li>
        );
      })}
    </ul>
  );
}

function GateRow({ kind, gate, now }: { kind: string; gate: Gate | undefined; now: number }) {
  const result = gate?.result ?? 'not_run';
  return (
    <>
      <div className="cm-row" data-gate={kind} data-result={result}>
        <span className="cm-gate-name"><span className={resultPip(result)} />{GATE_LABEL[kind] ?? kind}</span>
        <span className={`ap ${result === 'passed' ? 'origin' : result === 'failed' ? 'live' : 'faint'}`}>
          {RESULT_LABEL[result] ?? result}{gate && gate.ms ? ` · ${timeOf(gate.ms, now)}` : ''}
        </span>
      </div>
      {gate && result !== 'passed' && gate.findings.map((finding, index) => (
        <p key={index} className="cm-gate-finding">{finding.title}{finding.evidence ? `: ${finding.evidence}` : ''}</p>
      ))}
    </>
  );
}

function Card({ active }: { active?: boolean }) {
  const { selection, request, pack } = useOpen(active !== false);
  const now = useNow(60_000, active !== false);
  const data = request.data;
  if (selection.view !== 'session' || !selection.request) {
    return <div className="cm-pane" data-build={__CORTEX_BUILD__}><p className="cm-line flush">Open a request in the sidebar to see its card.</p></div>;
  }
  const gates = latestGates(pack.data?.gates ?? []).filter((gate) => gate.kind !== 'standard');
  const kinds = [...ALWAYS, ...gates.map((gate) => gate.kind).filter((kind) => !ALWAYS.includes(kind))];
  return (
    <div className="cm-pane" data-build={__CORTEX_BUILD__} data-request={selection.request}>
      <div className="cm-pane-h">
        <span className="ap">{data ? WORD_LABEL[data.word] ?? data.word : 'Card'}</span>
        <span className="grow" />
        {data && <span className={`tag ${data.tier}`}>{TIER_LABEL[data.tier] ?? data.tier}</span>}
        <span className="ap faint">{selection.request}</span>
      </div>
      {request.error && !data && <p className="inline-error">{loadFailure('The request', request)}</p>}
      {data && (
        <div className="cm-sec">
          <span className="ap">Stage</span>
          <Ladder request={data} pack={pack.data} now={now} />
          {data.ended_as && data.ended_as !== 'released' && <p className="cm-note faint">It ended: {data.ended_as}.</p>}
        </div>
      )}
      {pack.error && !pack.data && <p className="inline-error">{loadFailure('The package', pack)}</p>}
      {pack.data && (pack.data.requests.length > 1 || pack.data.attempt > 1 || pack.data.reworks > 0 || pack.data.urgent) && (
        <div className="cm-sec">
          <span className="ap">Package</span>
          {pack.data.requests.length > 1 && <div className="cm-row"><span>Built together</span><span>{count(pack.data.requests.length, 'request')}</span></div>}
          {(pack.data.attempt > 1 || pack.data.reworks > 0) && (
            <div className="cm-row"><span>Attempt</span><span className="num">{pack.data.attempt} · {count(pack.data.reworks, 'rework round')} used</span></div>
          )}
          {pack.data.urgent && <div className="cm-row"><span>Urgent</span><span className="ap live">No hold</span></div>}
        </div>
      )}
      {gates.length > 0 && (
        <div className="cm-sec">
          <span className="ap">Gates</span>
          {kinds.map((kind) => <GateRow key={kind} kind={kind} gate={gates.find((gate) => gate.kind === kind)} now={now} />)}
        </div>
      )}
    </div>
  );
}

export default definePlugin<{ active?: boolean }>({ slot: 'pane.card@1', component: local(Card) });
