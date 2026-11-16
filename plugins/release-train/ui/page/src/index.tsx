/**
 * page.releases@1: the Releases view of a repository. Tonight's train (the parts, now → next, the
 * requests each part carries), what soaks in the owner's apps, and what was released, with Take
 * back on each released package. The repository comes from the shared selection; everything shown
 * is `train.get`, read again on `train.changed`.
 */

import { definePlugin, errorMessage, useClient, useNow, useQuery, useShared } from '@cortex/sdk';
import { useState } from 'react';
import './page.css';

/** The workbench's shared selection (`manage.selection`). */
interface Selection {
  view: string;
  repo: string | null;
  request: string | null;
  package: string | null;
}

/** `[request id, title, tier]`. */
type Carried = [string, string, string];
interface Soak {
  package: string;
  repo: string;
  requests: Carried[];
  parts: string[];
  tier: string;
  urgent: boolean;
  commit: string;
  landed_ms: number;
  until_ms: number;
  reports?: string[];
  state: string;
}
interface PartMove { part: string; from: string; to: string; carries?: string[] }
interface Release {
  id: string;
  repo: string;
  ms: number;
  parts: PartMove[];
  packages: string[];
  note: string;
  state: string;
  tail?: string[];
  hotfix?: boolean;
}
interface TrainView { repo: string; next_ms: number; next: PartMove[]; soaking: Soak[]; released: Release[] }

const NO_SELECTION: Selection = { view: 'inbox', repo: null, request: null, package: null };
const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

const count = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;
const time = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const day = (ms: number) => new Date(ms).toLocaleDateString([], { day: 'numeric', month: 'short' });
const hours = (ms: number) => Math.max(0, Math.round(ms / HOUR_MS));

/** A request as a part's `carries` names it: its title, or its id when it has none. */
const carriedAs = (request: Carried) => request[1].trim() || request[0];
/** What a package is called on a row: its requests' titles, or its id. */
const titleOf = (soak: Soak) => soak.requests.map(carriedAs).join(' · ') || soak.package;

function Tier({ tier }: { tier: string }) {
  return tier ? <span className={`rt-tag ${tier}`}>{tier}</span> : null;
}

/** The train that leaves next: its parts and the two presses. */
function NextTrain({ view, now, onError }: { view: TrainView; now: number; onError: (text: string) => void }) {
  const client = useClient();
  const [busy, setBusy] = useState<'run' | 'skip' | null>(null);
  const [last, setLast] = useState<Release | null>(null);
  const ready = view.soaking.filter((soak) => soak.state === 'ready');
  // The tier of each carried request, by the name the part carries it under.
  const tiers = new Map<string, string>();
  for (const soak of ready) for (const request of soak.requests) tiers.set(carriedAs(request), request[2]);
  // A train more than a day away means tonight's was skipped.
  const skipped = view.next_ms - now > DAY_MS;

  const press = async (what: 'run' | 'skip') => {
    setBusy(what);
    onError('');
    try {
      if (what === 'run') setLast(await client.call<Release>('train.run', { repo: view.repo }));
      else await client.call('train.skip', { repo: view.repo });
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="rt-block">
      <div className="rt-hd">
        <span className="pip hollow" />
        <span className="ap">Tonight&rsquo;s train</span>
        <span className="ap faint">· {count(ready.length, 'package')} · {count(view.next.length, 'part')}</span>
        <span className="grow" />
        <span className="ap faint">{skipped ? `Skipped · next ${day(view.next_ms)} ${time(view.next_ms)}` : `Leaves ${time(view.next_ms)}`}</span>
      </div>
      {view.next.length === 0 ? (
        <p className="rt-none">Nothing is ready for the train yet.</p>
      ) : (
        <table className="tbl">
          <thead>
            <tr><th>Part</th><th>Now</th><th>Next</th><th>Carries</th></tr>
          </thead>
          <tbody>
            {view.next.map((part) => (
              <tr key={part.part}>
                <td className="k">{part.part}</td>
                <td><span className="num">{part.from || '—'}</span></td>
                <td><span className="num">{part.to || '—'}</span></td>
                <td>
                  {(part.carries ?? []).length === 0 ? <span className="rt-sub">Changed on the main line</span> : null}
                  {(part.carries ?? []).map((title, n) => (
                    <span className="rt-carry" key={`${title}-${n}`}>{title}<Tier tier={tiers.get(title) ?? ''} /></span>
                  ))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="rt-acts">
        <button className="detent" type="button" disabled={busy !== null || view.next.length === 0} onClick={() => void press('run')}>
          {busy === 'run' ? 'Releasing' : 'Release now'}
        </button>
        <button className="detent" type="button" disabled={busy !== null || skipped} onClick={() => void press('skip')}>Skip tonight</button>
        {last ? <span className={`ap ${last.state === 'failed' ? 'live' : 'faint'}`}>{outcome(last)}</span> : null}
      </div>
      {last?.state === 'failed' ? <pre className="rt-tail">{(last.tail ?? []).slice(-6).join('\n')}</pre> : null}
    </section>
  );
}

/** What a train did, in a few words. */
function outcome(release: Release): string {
  switch (release.state) {
    case 'ran': return 'Released';
    case 'nothing': return 'Nothing to release';
    case 'skipped': return 'Skipped';
    default: return 'Did not finish';
  }
}

/** Where a package stands in its soak. */
function soakLine(soak: Soak, now: number): string {
  const reports = soak.reports?.length ?? 0;
  const said = reports === 0 ? 'no reports' : count(reports, 'report');
  if (soak.state === 'ready') return `Soak done · ${said}`;
  const total = hours(soak.until_ms - soak.landed_ms);
  const done = Math.min(total, hours(now - soak.landed_ms));
  return `${done} h of ${total} · ${said}`;
}

function Soaking({ soaks, now }: { soaks: Soak[]; now: number }) {
  return (
    <section className="rt-block">
      <div className="rt-hd"><span className="pip done" /><span className="ap">Soaking in your apps</span></div>
      {soaks.length === 0 ? <p className="rt-none">Nothing is soaking.</p> : null}
      {soaks.map((soak) => (
        <div className="rt-row" key={soak.package}>
          <div className="what">
            {titleOf(soak)}<Tier tier={soak.urgent ? 'urgent' : soak.tier} />
            {(soak.reports ?? []).map((report, n) => <p className="rt-sub" key={n}>{report}</p>)}
          </div>
          <span className="side">
            {soak.state === 'held_back' ? <span className="ap live">Held back</span> : null}
            <span className="ap faint">{soakLine(soak, now)}</span>
          </span>
        </div>
      ))}
    </section>
  );
}

function Released({ releases, takenBack, onError }: { releases: Release[]; takenBack: Set<string>; onError: (text: string) => void }) {
  const client = useClient();
  const [taking, setTaking] = useState<string | null>(null);
  const [taken, setTaken] = useState<string[]>([]);

  const takeBack = async (id: string) => {
    setTaking(id);
    onError('');
    try {
      await client.call('train.take_back', { package: id });
      setTaken((was) => [...was, id]);
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setTaking(null);
    }
  };

  return (
    <section className="rt-block">
      <div className="rt-hd"><span className="pip done" /><span className="ap">Released</span></div>
      {releases.length === 0 ? <p className="rt-none">Nothing was released yet.</p> : null}
      {releases.map((release) => {
        const shipped = release.state === 'ran' || release.state === 'nothing';
        const parts = release.parts.map((part) => `${part.part} ${part.to}`.trim()).join(', ');
        return (
          <div className="rt-row" key={release.id}>
            <div className="what">
              {day(release.ms)} · {release.state === 'ran' && parts ? parts : outcome(release)}
              {release.hotfix ? <span className="rt-tag urgent">Hotfix</span> : null}
              {release.state === 'failed' && release.tail?.length ? <pre className="rt-tail">{release.tail.slice(-4).join('\n')}</pre> : null}
            </div>
            <span className="side">
              {shipped ? release.packages.map((id) => (
                takenBack.has(id) || taken.includes(id) ? (
                  <span className="ap faint" key={id}>{release.packages.length > 1 ? `${id} taken back` : 'Taken back'}</span>
                ) : (
                  <button className="detent" type="button" key={id} disabled={taking !== null} onClick={() => void takeBack(id)}>
                    {release.packages.length > 1 ? `Take back ${id}` : 'Take back'}
                  </button>
                )
              )) : <span className="ap faint">{count(release.packages.length, 'package')} waiting</span>}
            </span>
          </div>
        );
      })}
    </section>
  );
}

function ReleasesPage() {
  const [selection] = useShared<Selection>('manage.selection', NO_SELECTION);
  const repo = selection?.repo ?? null;
  const train = useQuery<TrainView>(repo ? `train.get:${repo}` : null, (client) => client.call<TrainView>('train.get', { repo }), { topics: ['train.changed'] });
  const now = useNow(60_000, repo !== null);
  const [error, setError] = useState('');

  if (!repo) {
    return <div className="rt-page"><p className="rt-none">Choose a repository to see its releases.</p></div>;
  }
  const view = train.data;
  if (!view) {
    return <div className="rt-page"><p className="rt-none">{train.error ? errorMessage(train.error) : 'Reading the train.'}</p></div>;
  }
  const waiting = view.soaking.filter((soak) => soak.state !== 'taken_back');
  const takenBack = new Set(view.soaking.filter((soak) => soak.state === 'taken_back').map((soak) => soak.package));
  return (
    <div className="rt-page">
      {error ? <span className="ap live" role="alert">{error}</span> : null}
      <NextTrain view={view} now={now} onError={setError} />
      <Soaking soaks={waiting} now={now} />
      <Released releases={view.released} takenBack={takenBack} onError={setError} />
    </div>
  );
}

export default definePlugin({ slot: 'page.releases@1', component: ReleasesPage });
