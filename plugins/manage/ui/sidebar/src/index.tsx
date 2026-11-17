/**
 * workbench.sidebar@1 — at the top New request (it opens the conversation in the centre) and Add
 * repo (a sheet: from GitHub, or any git address). Then the
 * repositories, and under each its open requests: a package of several requests is a bracket
 * around them. Below, under its own heading, the shared standard, only when the person's
 * repositories carry one.
 * Pressing a repository's name opens its page; pressing a request opens its session.
 */

import { useState } from 'react';
import { Cross, definePlugin, useNow, useQuery, type Client } from '@cortex/sdk';
import {
  CHANGED,
  STAGE_LABEL,
  count,
  loadFailure,
  local,
  pipClass,
  sessionOf,
  shortStage,
  timeOf,
  useManageSelection,
  type PackageView,
  type Repo,
  type RequestView,
  type Selection,
} from './manage';
import { AddRepo } from './AddRepo';
import './manage.css';
import './sidebar.css';

const listRepos = async (client: Client) => (await client.call<{ repos: Repo[] }>('manage.repo.list', {}))?.repos ?? [];

function useRepos() {
  return useQuery<Repo[]>('manage.repo.list', listRepos, { topics: [CHANGED] });
}

/** What the bracket around a package's requests says. */
function packageLabel(pack: PackageView | undefined, now: number): string {
  if (!pack) return 'Package';
  if (pack.stage === 'held') {
    if (pack.stopped) return 'Package · stopped';
    return pack.hold_until_ms ? `Package · held until ${timeOf(pack.hold_until_ms, now)}` : 'Package · waits for your yes';
  }
  return `Package · ${(STAGE_LABEL[pack.stage] ?? pack.stage).toLowerCase()}`;
}

/** The requests in their order, those of one package of several gathered where its first one is. */
function grouped(requests: RequestView[]): Array<{ pack: string | null; requests: RequestView[] }> {
  const size = new Map<string, number>();
  for (const request of requests) if (request.package) size.set(request.package, (size.get(request.package) ?? 0) + 1);
  const out: Array<{ pack: string | null; requests: RequestView[] }> = [];
  const brackets = new Map<string, RequestView[]>();
  for (const request of requests) {
    if (request.package && (size.get(request.package) ?? 0) > 1) {
      let members = brackets.get(request.package);
      if (!members) {
        members = [];
        brackets.set(request.package, members);
        out.push({ pack: request.package, requests: members });
      }
      members.push(request);
    } else {
      const last = out[out.length - 1];
      if (last && last.pack === null) last.requests.push(request);
      else out.push({ pack: null, requests: [request] });
    }
  }
  return out;
}

/** A row's word at its right. An urgent request says so until it has landed. */
function stageWord(request: RequestView): string {
  const landed = request.stage === 'landed' || request.stage === 'soaking' || request.stage === 'released';
  if (request.urgent && !landed && request.word !== 'needs_you' && request.word !== 'stuck' && request.word !== 'on_hold') return 'Urgent';
  return shortStage(request);
}

function Sessions({ requests, selection, open }: { requests: RequestView[]; selection: Selection; open: (next: Selection) => void }) {
  return (
    <ul className="cm-sess">
      {requests.map((request) => {
        const selected = selection.view === 'session' && selection.request === request.id;
        const attention = request.word === 'needs_you' || request.word === 'stuck';
        return (
          <li key={request.id} className={`${selected ? 'sel' : ''}${attention ? ' attn' : ''}`} data-request={request.id}>
            <button type="button" title={`${request.id} · ${request.title}`} aria-pressed={selected} onClick={() => open(sessionOf(request))}>
              <span className={pipClass(request.word)} />
              <span className="tt">{request.title || request.id}</span>
              <span className="st">{stageWord(request)}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function RepoBlock({ repo, index, selection, open, now }: { repo: Repo; index: number; selection: Selection; open: (next: Selection) => void; now: number }) {
  const requests = useQuery<RequestView[]>(
    `manage.request.list:${repo.id}`,
    async (client) => (await client.call<{ requests: RequestView[] }>('manage.request.list', { repo: repo.id }))?.requests ?? [],
    { topics: [CHANGED] },
  );
  const packages = useQuery<PackageView[]>(
    `manage.package.list:${repo.id}`,
    async (client) => (await client.call<{ packages: PackageView[] }>('manage.package.list', { repo: repo.id }))?.packages ?? [],
    { topics: [CHANGED] },
  );
  const open_ = requests.data ?? [];
  const on = selection.repo === repo.id && selection.view !== 'inbox' && selection.view !== 'standard' && selection.view !== 'new';
  const sub = repo.managed
    ? `builds on ${repo.runner_name || 'this computer'}${requests.data ? ` · ${open_.length === 0 ? 'no open request' : count(open_.length, 'open request')}` : ''}`
    : 'not managed yet';
  return (
    <section className={`cm-repo${repo.managed ? '' : ' off'}`} data-repo={repo.id}>
      <ul className="nav">
        <li className={on ? 'on' : undefined}>
          <button type="button" title={`${repo.name}: charter, releases and runner`} onClick={() => open({ view: 'charter', repo: repo.id, request: null, package: null })}>
            <span className="kk">{String(index + 1).padStart(2, '0')}</span>
            <span className="nm">{repo.name}</span>
            <span className="sub">{sub}</span>
          </button>
        </li>
      </ul>
      {requests.error && !requests.data && <p className="inline-error">{loadFailure(`The requests of ${repo.name}`, requests)}</p>}
      {packages.error && !packages.data && <p className="inline-error">{loadFailure(`The packages of ${repo.name}`, packages)}</p>}
      {grouped(open_).map((group) => (group.pack ? (
        <div key={group.pack} className="cm-pk" data-package={group.pack}>
          <span className="ap origin">{packageLabel(packages.data?.find((pack) => pack.id === group.pack), now)}</span>
          <Sessions requests={group.requests} selection={selection} open={open} />
        </div>
      ) : (
        <Sessions key={group.requests[0]!.id} requests={group.requests} selection={selection} open={open} />
      )))}
    </section>
  );
}

/** The standard's last report, as much of it as this row reads. */
interface StandardReport { version: string; drift: string[] }

function StandardRow({ selection, open }: { selection: Selection; open: (next: Selection) => void }) {
  const report = useQuery<StandardReport>('standard.get', (client) => client.call<StandardReport>('standard.get', {}), { topics: ['standard.changed'] });
  const drift = report.data?.drift?.length ?? 0;
  let sub: string;
  if (report.data) {
    const version = report.data.version ? `version ${report.data.version}` : 'not locked yet';
    sub = `${version} · ${drift === 0 ? 'the same in every app' : `${count(drift, 'app')} ${drift === 1 ? 'differs' : 'differ'}`}`;
  } else if (report.error) {
    sub = report.errorCode === 'unavailable' ? 'the standard part is not running' : `not known: ${report.error}`;
  } else {
    sub = 'checking';
  }
  return (
    <ul className="nav">
      <li className={selection.view === 'standard' ? 'on' : undefined}>
        <button type="button" title="The files every Cortex app must carry unchanged, and which app's copy differs" onClick={() => open({ view: 'standard', repo: null, request: null, package: null })}>
          <span className={`kk${drift > 0 ? ' drift' : ''}`}>{drift > 0 ? '≠' : '='}</span>
          <span className="nm">Mesh standard</span>
          <span className="sub">{sub}</span>
        </button>
      </li>
    </ul>
  );
}

/** The shared standard, only for a person whose repositories carry one: with no member of a
 * standard there is nothing to show, and nothing is shown. */
function SharedStandard({ selection, open }: { selection: Selection; open: (next: Selection) => void }) {
  const members = useQuery<unknown[]>('standard.member.list', async (client) => (await client.call<{ members: unknown[] }>('standard.member.list', {}))?.members ?? [], { topics: ['standard.changed'] });
  if (!members.data || members.data.length === 0) return null;
  return (
    <>
      <div className="index-h cm-side-shared"><Cross /><span className="ap">Shared by every app</span></div>
      <StandardRow selection={selection} open={open} />
    </>
  );
}

function Sidebar() {
  const repos = useRepos();
  const [selection, setSelection] = useManageSelection();
  const [form, setForm] = useState<'repo' | null>(null);
  // Times in the brackets turn from a clock into a day at midnight.
  const now = useNow(60_000, true);
  const list = repos.data ?? [];
  const done = (next: Selection | null) => {
    setForm(null);
    if (next) setSelection(next);
    void repos.refresh();
  };
  return (
    <div className="cm-side" data-build={__CORTEX_BUILD__}>
      <div className="cm-side-acts">
        <button type="button" className="detent" aria-pressed={selection.view === 'new'} title="Describe a change to a repository; it becomes a session under that repository"
          onClick={() => { setForm(null); setSelection({ view: 'new', repo: selection.repo ?? list.find((repo) => repo.managed)?.id ?? null, request: null, package: null }); }}>New request</button>
        <button type="button" className="detent" aria-pressed={form === 'repo'} title="Add a git repository for CortexManage to manage" onClick={() => setForm(form === 'repo' ? null : 'repo')}>Add repo</button>
      </div>
      {form === 'repo' && <AddRepo onClose={() => setForm(null)} onAdded={(next) => done(next)} />}
      <div className="index-h"><Cross /><span className="ap">Repositories</span></div>
      <div className="cm-side-scroll">
        {repos.error && !repos.data && <p className="inline-error">{loadFailure('The repositories', repos)}</p>}
        {repos.data && list.length === 0 && <p className="cm-line flush">No repository yet. Add one with Add repo.</p>}
        {list.map((repo, index) => <RepoBlock key={repo.id} repo={repo} index={index} selection={selection} open={setSelection} now={now} />)}
        <SharedStandard selection={selection} open={setSelection} />
      </div>
    </div>
  );
}

export default definePlugin({ slot: 'workbench.sidebar@1', component: local(Sidebar) });
