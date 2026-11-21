/**
 * pane.card@1 — Details of the open request: what it is and who reported it, one line on where
 * it is, a decision when one waits on the person, and its milestones as a line of dots; pressing
 * a dot shows that moment in plain words. No code, no file names, no commands.
 */

import { useState, type ReactNode } from 'react';
import { definePlugin, setShared, useClient, useNow, useQuery } from '@cortex/sdk';
import { CHANGED, LAYOUT_BOARD, NO_SELECTION, TIER_LABEL, loadFailure, local, timeOf, useAct, useManageSelection, useOpen, type Ask, type BoardItem, type PackageView, type RequestView } from './manage';
import './manage.css';
import './card.css';

const cap = (text: string) => (text ? text[0]!.toUpperCase() + text.slice(1) : text);

/** The milestones, in order, and the stages that reach each. */
const MILESTONES = [
  { key: 'in', label: 'Came in', stages: ['arrived'] },
  { key: 'read', label: 'Understood', stages: ['clarifying', 'assessed'] },
  { key: 'plan', label: 'Planned', stages: ['planned', 'validated'] },
  { key: 'wait', label: 'Waiting', stages: ['held'] },
  { key: 'code', label: 'Coding', stages: ['building'] },
  { key: 'check', label: 'Checked', stages: ['proving'] },
  { key: 'done', label: 'Done', stages: ['landed', 'soaking'] },
  { key: 'ship', label: 'Shipped', stages: ['released'] },
] as const;
type Key = (typeof MILESTONES)[number]['key'];

const indexOfStage = (stage: string) => Math.max(0, MILESTONES.findIndex((m) => (m.stages as readonly string[]).includes(stage)));

/** When a milestone was last reached (0: not yet). */
function reachedAt(request: RequestView, key: Key): number {
  const stages = MILESTONES.find((m) => m.key === key)!.stages as readonly string[];
  let at = 0;
  for (const [stage, ms] of request.stages ?? []) if (stages.includes(stage)) at = ms;
  return at;
}

const TIER_SAYS: Record<string, string> = {
  safe: 'Nobody notices a difference.',
  minor: 'People notice the difference; nothing is redesigned.',
  major: 'It changes how things are built, so it waits for your yes.',
};

/** The one sentence on where the work is. */
function nowLine(request: RequestView, item: BoardItem | undefined, pack: PackageView | undefined, ask: Ask | undefined, now: number): ReactNode {
  if (request.ended_as && request.ended_as !== 'released') return <><b>Closed.</b> {cap(request.ended_as)}.</>;
  if (ask) return <b>Waits for your yes.</b>;
  switch (item?.column) {
    case 'incoming':
      return request.stage === 'arrived' ? <b>Reading it.</b> : request.stage === 'assessed' ? <b>Planning it.</b> : <b>Checking the plan.</b>;
    case 'waiting':
      if (item.paused) return <><b>Paused.</b> Drag it to Coding to go on.</>;
      return item.starts_ms ? <><b>Starts {timeOf(item.starts_ms, now)}</b>{item.together ? ', together with other work.' : '.'}</> : <b>Waiting.</b>;
    case 'coding':
      if (item.queued === 'after') return <b>Starts after the work being coded now.</b>;
      if (item.queued === 'builder') return <b>Waiting for a CortexCode to take it.</b>;
      if (item.queued === 'next') return <b>Starting.</b>;
      if (item.checking) return <b>Checking that it works.</b>;
      return <><b>Step {Math.max(1, item.step)} of {item.steps || '?'}</b>{pack?.runner_name ? ` on ${pack.runner_name}.` : '.'}</>;
    case 'done':
      return item.release_ms ? <><b>Done.</b> Releases {timeOf(item.release_ms, now)}.</> : <><b>Done.</b> Waiting for the next release.</>;
    case 'shipped':
      return <><b>Shipped</b>{request.ended_reason ? ` in ${request.ended_reason}` : ''}{item.released_ms ? `, ${timeOf(item.released_ms, now)}` : ''}.</>;
    default:
      return null;
  }
}

/** What one milestone says. */
function Moment({ k, request, item, pack, items, now }: { k: Key; request: RequestView; item: BoardItem | undefined; pack: PackageView | undefined; items: BoardItem[]; now: number }) {
  const points = request.plan?.points ?? [];
  const kept = points.filter((point) => point.verdict === 'keep' || point.verdict === '');
  const cut = points.filter((point) => point.verdict === 'cut');
  const planned = (request.stages ?? []).filter(([stage]) => stage === 'planned').length;
  const step = pack?.strip?.task ?? item?.step ?? 0;
  const results = new Map((pack?.evidence ?? []).map((evidence) => [evidence.walk, evidence]));
  const steered = (pack?.interventions ?? []).filter((one) => one.action === 'steer' || one.action === 'restart').length;
  const others = pack ? pack.requests.filter((id) => id !== request.id).map((id) => items.find((each) => each.id === id)?.title ?? id) : [];
  const finished = ['proving', 'landed', 'soaking', 'released'].includes(request.stage);
  switch (k) {
    case 'in': {
      const reporters = item?.reporters ?? [];
      return (
        <>
          <q>{request.text.length > 500 ? `${request.text.slice(0, 500)}…` : request.text}</q>
          {item && item.reports > 0 ? (
            <p>Reported {item.reports + 1} times{reporters.length ? `: ${reporters.join(', ')}` : ''}. The later reports were the same problem, so they were added to this one.</p>
          ) : (
            <p>{reporters.length ? `Reported by ${reporters.join(', ')}.` : 'You asked for it.'}</p>
          )}
        </>
      );
    }
    case 'read': {
      const reason = request.tier_reasons?.[0] ?? '';
      const why = reason.includes(':') ? reason.slice(reason.indexOf(':') + 1).trim() : '';
      return request.kind ? (
        <>
          <p><b>{cap(request.kind)}, {TIER_LABEL[request.tier].toLowerCase()}.</b> {TIER_SAYS[request.tier]}</p>
          {why && <p>{why}</p>}
        </>
      ) : <p>Being read now.</p>;
    }
    case 'plan':
      return kept.length ? (
        <>
          {planned > 1 && <p><b>Planned {planned} times.</b> The earlier ways did not work, so a different one was planned.</p>}
          <ol>{kept.map((point) => <li key={point.n}>{point.title}</li>)}</ol>
          {kept.some((point) => point.walks.length) && (
            <>
              <p className="ap faint cm-d-sub">How we will know it works</p>
              <ul className="cm-d-checks">
                {kept.flatMap((point) => point.walks).map((walk) => <li key={walk.id}><span className="r">To check</span><span>{walk.do} → {walk.see}</span></li>)}
              </ul>
            </>
          )}
          {cut.map((point) => <p key={point.n} className="cm-d-small">Left out: {point.title}{point.verdict_reason ? `: ${point.verdict_reason}` : ''}</p>)}
        </>
      ) : <p>Being planned now.</p>;
    case 'wait':
      return (
        <>
          {item?.paused ? <p><b>Paused.</b> It waits until you drag it to Coding.</p>
            : pack && pack.hold_until_ms === 0 && pack.stage === 'held' ? <p>Waiting for your yes before coding.</p>
            : <p>Waiting so related requests can join.{item?.starts_ms ? <> <b>Starts {timeOf(item.starts_ms, now)}</b> by itself; drag it to Coding to start now.</> : null}</p>}
          {others.length > 0 && <p>Built together with {others.map((title, n) => <span key={title}>{n ? ', ' : ''}<b>“{title}”</b></span>)}.</p>}
        </>
      );
    case 'code':
      return (
        <>
          <p>A CortexCode{pack?.runner_name ? <> on <b>{pack.runner_name}</b></> : null} {finished ? 'built it.' : 'is on it.'}{!finished && step ? <> <b>Step {step} of {kept.length || item?.steps}</b>.</> : null}</p>
          <ol>{kept.map((point, index) => <li key={point.n} className={finished || index + 1 < step ? 'd' : index + 1 === step ? 'n' : undefined}>{point.title}</li>)}</ol>
          {steered > 0 && <p className="cm-d-small">It went off its step {steered === 1 ? 'once' : `${steered} times`} and was put back. Nothing needed you.</p>}
        </>
      );
    case 'check': {
      const walks = kept.flatMap((point) => point.walks);
      return (
        <>
          <ul className="cm-d-checks">
            {walks.map((walk) => {
              const seen = results.get(walk.id);
              const label = seen?.result === 'passed' ? 'Passed' : seen?.result === 'failed' ? 'Failed' : seen ? 'Not run' : 'To check';
              return <li key={walk.id}><span className={`r${seen?.result === 'passed' ? ' ok' : seen?.result === 'failed' ? ' bad' : ''}`}>{label}</span><span>{walk.do} → {seen?.seen || walk.see}</span></li>;
            })}
          </ul>
          {(pack?.reworks ?? 0) > 0 && <p className="cm-d-small">A check failed at first; it was fixed and checked again by itself.</p>}
        </>
      );
    }
    case 'done':
      return (
        <>
          <p>On the main branch. <b>{item?.release_ms ? `Goes out with the next release, ${timeOf(item.release_ms, now)}.` : 'Goes out with the next release.'}</b></p>
          {item && item.reports > 0 && <p>{item.reports + 1} reports close with it{item.reporters.length ? `: ${item.reporters.join(', ')}` : ''}.</p>}
        </>
      );
    case 'ship':
      return (
        <>
          <p><b>Released{request.ended_reason ? ` in ${request.ended_reason}` : ''}</b>{item?.released_ms ? `, ${timeOf(item.released_ms, now)}` : ''}.</p>
          <p>If it is reported again, the report is matched to this and answered: already fixed, update the app.</p>
        </>
      );
  }
}

/** The milestones as a line of dots; the newest reached is open by default. */
function Line({ request, reached, chosen, choose, alerts }: { request: RequestView; reached: number; chosen: Key; choose: (key: Key) => void; alerts: Set<Key> }) {
  const w = 384;
  const pad = 16;
  const step = (w - pad * 2) / (MILESTONES.length - 1);
  const x = (i: number) => pad + i * step;
  return (
    <svg className="cm-d-line" viewBox={`0 0 ${w} 52`} role="list" aria-label="What happened">
      <line className="axis" x1={x(0)} y1={22} x2={x(MILESTONES.length - 1)} y2={22} />
      <line className="axis done" x1={x(0)} y1={22} x2={x(reached)} y2={22} />
      {MILESTONES.map((m, i) => {
        const at = reachedAt(request, m.key);
        const state = i < reached ? 'past' : i === reached ? 'here' : 'ahead';
        const cls = `mark ${alerts.has(m.key) && i < reached ? 'alert' : state}${chosen === m.key ? ' on' : ''}`;
        return (
          <g key={m.key} role="listitem">
            <circle
              className={cls}
              cx={x(i)}
              cy={22}
              r={i === reached ? 6 : 4.5}
              tabIndex={i <= reached ? 0 : -1}
              onClick={() => i <= reached && choose(m.key)}
              onKeyDown={(event) => { if ((event.key === 'Enter' || event.key === ' ') && i <= reached) choose(m.key); }}
            >
              <title>{m.label}{at ? ` · ${timeOf(at)}` : ''}</title>
            </circle>
            {i === reached && <text className="lbl" x={x(i)} y={8} textAnchor="middle">{m.label}</text>}
            {i <= reached && at > 0 && <text className="when" x={x(i)} y={44} textAnchor="middle">{timeOf(at).split(' ').pop()}</text>}
          </g>
        );
      })}
    </svg>
  );
}

function Details({ active }: { active?: boolean }) {
  const { selection, request, pack } = useOpen(active !== false);
  const now = useNow(60_000, active !== false);
  const client = useClient();
  const act = useAct();
  const [, setSelection] = useManageSelection();
  const board = useQuery<{ items: BoardItem[]; asks: Ask[] }>(
    active !== false ? 'manage.board.items' : null,
    async (c) => (await c.call<{ items: BoardItem[]; asks: Ask[] }>('manage.board.items', {})) ?? { items: [], asks: [] },
    { topics: [CHANGED, 'train.changed'] },
  );
  const [picked, setPicked] = useState<{ id: string; key: Key } | null>(null);
  const data = request.data;
  if (selection.view !== 'session' || !selection.request) {
    return <div className="cm-pane" data-build={__CORTEX_BUILD__}><p className="cm-line flush">Press a request on the board.</p></div>;
  }
  if (!data) {
    return <div className="cm-pane" data-build={__CORTEX_BUILD__}>{request.error ? <p className="inline-error">{loadFailure('The request', request)}</p> : null}</div>;
  }
  const items = board.data?.items ?? [];
  const item = items.find((each) => each.id === data.id);
  const p = pack.data;
  const ask = (board.data?.asks ?? []).find((each) => each.request === data.id || (p !== undefined && each.package === p.id));
  const reached = indexOfStage(data.stage);
  const pickedIndex = picked && picked.id === data.id ? MILESTONES.findIndex((m) => m.key === picked.key) : -1;
  const chosen: Key = pickedIndex >= 0 && pickedIndex <= reached ? MILESTONES[pickedIndex]!.key : MILESTONES[reached]!.key;
  const alerts = new Set<Key>();
  if ((p?.evidence ?? []).some((evidence) => evidence.result === 'failed') || (p?.reworks ?? 0) > 0) alerts.add('check');
  if ((data.stages ?? []).filter(([stage]) => stage === 'planned').length > 1) alerts.add('plan');
  const tags = data.kind ? [cap(data.kind), TIER_LABEL[data.tier], data.surface.map(cap).join(' + ')].filter(Boolean).join(' · ') : '';
  const reporters = item?.reporters ?? [];
  const from = item && item.reports > 0 ? `×${item.reports + 1} reports${reporters.length ? ` · ${reporters.join(', ')}` : ''}` : reporters.length ? `From ${reporters.join(', ')}` : '';
  const edit = (params: Record<string, unknown>, doing: string) => void act.run('edit', doing, () => client.call('manage.package.edit', params));
  const decide = (action: string) => {
    if (ask) void act.run('decide', 'Answering', () => client.call('manage.decide', { id: ask.id, attempt: ask.attempt, action, note: '' }));
  };
  const milestone = MILESTONES.find((m) => m.key === chosen)!;
  const at = reachedAt(data, chosen);
  return (
    <div className="cm-pane cm-d" data-build={__CORTEX_BUILD__} data-request={data.id}>
      <div className="cm-d-head">
        <h3>{data.title}</h3>
        <div className="meta">
          {tags && <span className="ap faint">{tags}</span>}
          {from && <span className="ap from">{from}</span>}
        </div>
      </div>
      <div className="cm-d-now">
        <p className="say">{nowLine(data, item, p, ask, now)}</p>
        <div className="acts">
          <button type="button" className="detent" onClick={() => setShared(LAYOUT_BOARD, false)}>Open chat</button>
          {p && item?.column === 'waiting' && !ask && <button type="button" className="detent" disabled={act.busy !== null} onClick={() => edit({ id: p.id, start_now: true }, 'Starting it')}>Start now</button>}
          {p && item?.column === 'coding' && !item.queued && <button type="button" className="detent" disabled={act.busy !== null} onClick={() => edit({ id: p.id, stop: true }, 'Pausing it')}>Pause</button>}
          {item && (
            <button type="button" className="detent" data-action="cancel" disabled={act.busy !== null}
              onClick={() => void act.run('edit', item.column === 'done' || item.column === 'shipped' ? 'Deleting it' : 'Cancelling it', async () => {
                await client.call('manage.request.edit', { id: data.id, withdraw: true });
                setSelection(NO_SELECTION);
              })}>
              {item.column === 'done' || item.column === 'shipped' ? 'Delete' : 'Cancel'}
            </button>
          )}
        </div>
        {act.failed('edit') && <p className="inline-error">{act.failed('edit')}</p>}
        {ask && (
          <div className="cm-d-needs">
            <span className="ap">Needs you</span>
            <p><b>{ask.subject}</b> {ask.why}</p>
            {(ask.if_yes || ask.if_no) && (
              <dl>
                {ask.if_yes && <><dt>If yes</dt><dd>{ask.if_yes}</dd></>}
                {ask.if_no && <><dt>If no</dt><dd>{ask.if_no}</dd></>}
              </dl>
            )}
            <div className="acts">
              {ask.actions.includes('approve') && <button type="button" className="detent go" disabled={act.busy !== null} onClick={() => decide('approve')}>Approve</button>}
              {ask.actions.includes('send_back') && <button type="button" className="detent" onClick={() => setShared(LAYOUT_BOARD, false)}>Send back</button>}
              {ask.actions.includes('decline') && <button type="button" className="detent" disabled={act.busy !== null} onClick={() => decide('decline')}>Decline</button>}
            </div>
            {act.failed('decide') && <p className="inline-error">{act.failed('decide')}</p>}
          </div>
        )}
      </div>
      <div className="cm-d-linebox">
        <Line request={data} reached={reached} chosen={chosen} choose={(key) => setPicked({ id: data.id, key })} alerts={alerts} />
      </div>
      <div className="cm-d-moment">
        <div className="top"><span className="ap">{milestone.label}</span>{at > 0 && <time>{timeOf(at, now)}</time>}</div>
        <Moment k={chosen} request={data} item={item} pack={p} items={items} now={now} />
      </div>
    </div>
  );
}

export default definePlugin<{ active?: boolean }>({ slot: 'pane.card@1', component: local(Details) });
