/**
 * pane.evidence@1 — the Evidence tab, for the open request's package: every kept point with its
 * walks, each walk with its result, what to do and what is seen when it works, what was seen, how
 * it was walked and when; then the gates in one line. A walk that was blocked is not proven: it is
 * never drawn or counted as passed.
 */

import { definePlugin, useNow } from '@cortex/sdk';
import { GATE_LABEL, RESULT_LABEL, count, latestGates, loadFailure, local, resultPip, timeOf, useOpen, type Evidence, type Gate, type Point, type Walk } from './manage';
import './manage.css';
import './evidence.css';

const METHOD_LABEL: Record<string, string> = {
  computer: 'computer walk',
  command: 'command',
  mesh: 'mesh',
  journal: 'journal',
};

/** The newest evidence a walk left behind. */
function newestByWalk(evidence: Evidence[]): Map<string, Evidence> {
  const newest = new Map<string, Evidence>();
  for (const item of evidence) {
    const current = newest.get(item.walk);
    if (!current || item.ms >= current.ms) newest.set(item.walk, item);
  }
  return newest;
}

function WalkRow({ walk, left, now }: { walk: Walk; left: Evidence | undefined; now: number }) {
  const result = left?.result ?? 'not_run';
  const proven = result === 'passed';
  return (
    <div className="cm-walk" data-walk={walk.id} data-result={result}>
      <span className={resultPip(result)} />
      <p className="do"><span className="ap faint">Do</span>{walk.do}</p>
      <p><span className="ap faint">See</span>{walk.see}</p>
      {left?.seen && <p><span className="ap faint">Seen</span>{left.seen}</p>}
      <div className="how">
        <span className={`ap ${proven ? 'origin' : result === 'failed' ? 'live' : 'faint'}`}>{RESULT_LABEL[result] ?? result}</span>
        {result === 'blocked' && <span className="ap faint">not proven</span>}
        <span className="ap faint">
          {METHOD_LABEL[walk.method] ?? walk.method}{left?.kind ? ` · ${left.kind}` : ''}{left?.ms ? ` · ${timeOf(left.ms, now)}` : ''}{left?.build ? ` · build ${left.build.slice(0, 7)}` : ''}
        </span>
      </div>
    </div>
  );
}

function gateText(gate: Gate): string {
  const result = (RESULT_LABEL[gate.result] ?? gate.result).toLowerCase();
  const findings = gate.result !== 'passed' && gate.findings.length > 0 ? ` (${count(gate.findings.length, 'finding')})` : '';
  return `${GATE_LABEL[gate.kind] ?? gate.kind}: ${result}${findings}.`;
}

function Evidences({ active }: { active?: boolean }) {
  const { selection, request, pack, packageId } = useOpen(active !== false);
  const now = useNow(60_000, active !== false && Boolean(packageId));
  if (selection.view !== 'session' || !selection.request) {
    return <div className="cm-pane" data-build={__CORTEX_BUILD__}><p className="cm-line flush">Open a request in the sidebar to see its evidence.</p></div>;
  }
  const data = pack.data;
  const points: Point[] = data?.points ?? [];
  const newest = newestByWalk(data?.evidence ?? []);
  const walks = points.flatMap((point) => point.walks);
  const proven = walks.filter((walk) => newest.get(walk.id)?.result === 'passed').length;
  const failed = walks.filter((walk) => newest.get(walk.id)?.result === 'failed').length;
  const gates = latestGates(data?.gates ?? []);
  return (
    <div className="cm-pane" data-build={__CORTEX_BUILD__} data-package={packageId ?? ''}>
      <div className="cm-pane-h">
        <span className="ap">Evidence</span>
        {data && walks.length > 0 && <span className="ap faint">{proven} of {count(walks.length, 'walk')} proven{failed > 0 ? ` · ${failed} failed` : ''}</span>}
        <span className="grow" />
        <span className="ap faint">{packageId ?? selection.request}</span>
      </div>
      {request.error && !request.data && <p className="inline-error">{loadFailure('The request', request)}</p>}
      {pack.error && !data && <p className="inline-error">{loadFailure('The package', pack)}</p>}
      {request.data && !packageId && <p className="cm-line flush">This request is not in a package yet, so nothing has been walked.</p>}
      {data && points.length === 0 && <p className="cm-line flush">The package has no kept point yet.</p>}
      {points.map((point) => (
        <div key={`${point.request}:${point.n}`} className="cm-point" data-point={point.n}>
          <div className="cm-point-h"><span className="kk">{point.n}</span><span>{point.title}</span></div>
          {point.walks.length === 0 && <div className="cm-walk"><span className="cx-pip" /><p>No walk was written for this point, so it is not proven.</p></div>}
          {point.walks.map((walk) => <WalkRow key={walk.id} walk={walk} left={newest.get(walk.id)} now={now} />)}
        </div>
      ))}
      {data && (
        <div className="cm-sec">
          <span className="ap">Gates</span>
          <p className="cm-note">{gates.length === 0 ? 'No gate has run yet.' : gates.map(gateText).join(' ')}</p>
        </div>
      )}
    </div>
  );
}

export default definePlugin<{ active?: boolean }>({ slot: 'pane.evidence@1', component: local(Evidences) });
