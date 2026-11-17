/**
 * workbench.sidebar@1 — at the top New request and Add repo, each a small form in place. Then the
 * repositories, and under each its open requests: a package of several requests is a bracket
 * around them. Below, under its own heading, the code the Cortex apps share (the mesh standard).
 * Pressing a repository's name opens its page; pressing a request opens its session.
 */

import { useState } from 'react';
import { Cross, Menu, definePlugin, useClient, useNow, useQuery, type Client } from '@cortex/sdk';
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
  useAct,
  useManageSelection,
  type PackageView,
  type Repo,
  type RequestView,
  type Selection,
} from './manage';
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
  const on = selection.repo === repo.id && selection.view !== 'inbox' && selection.view !== 'standard';
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

function NewRequest({ repos, selection, done }: { repos: Repo[]; selection: Selection; done: (next: Selection | null) => void }) {
  const client = useClient();
  const act = useAct();
  const managed = repos.filter((repo) => repo.managed);
  const [repoId, setRepoId] = useState<string | null>(() => (managed.some((repo) => repo.id === selection.repo) ? selection.repo : managed[0]?.id ?? null));
  const [text, setText] = useState('');
  const [urgent, setUrgent] = useState(false);
  const repo = managed.find((candidate) => candidate.id === repoId) ?? null;
  const send = () => {
    if (!repo || !text.trim()) return;
    void act.run('send', 'Sending the request', async () => {
      // `urgent` is only sent when the person says so; otherwise the assessment decides.
      const created = await client.call<RequestView>('manage.request.create', {
        repo: repo.id,
        text: text.trim(),
        from: { kind: 'person' },
        ...(urgent ? { urgent: true } : {}),
      });
      done(sessionOf(created));
    });
  };
  return (
    <div className="cm-side-form cm-form" data-form="request">
      <span className="ap">New request</span>
      {managed.length === 0 ? (
        <p className="cm-line flush">Add a repository first: a request is always for one.</p>
      ) : (
        <>
          <div className="cm-form-row">
            <Menu label="Repository" value={repo?.name ?? 'choose'} items={managed.map((candidate) => ({
              key: candidate.id, label: candidate.name, note: candidate.prefix, selected: candidate.id === repoId, onSelect: () => setRepoId(candidate.id),
            }))} />
          </div>
          <textarea className="cm-area" value={text} placeholder="What should change, in your own words" aria-label="The request" onChange={(event) => setText(event.target.value)} />
          <div className="cm-form-row">
            <button type="button" className="sw" aria-pressed={urgent} aria-label="Urgent" onClick={() => setUrgent(!urgent)} />
            <span className="ap faint">Urgent</span>
            <span className="grow" />
            <button type="button" className="detent" onClick={() => done(null)}>Cancel</button>
            <button type="button" className="detent send" aria-pressed={Boolean(repo && text.trim())} disabled={!repo || !text.trim() || act.busy !== null} onClick={send}>Send</button>
          </div>
        </>
      )}
      {managed.length === 0 && <div className="cm-form-row"><span className="grow" /><button type="button" className="detent" onClick={() => done(null)}>Close</button></div>}
      {act.failed('send') && <p className="inline-error">{act.failed('send')}</p>}
    </div>
  );
}

/** A joined computer as `mesh.peers` answers it (the fields the runner menu reads). */
interface Peer { member: { node: string; name: string }; card?: { software?: string } | null; reachable: boolean }

function useRunners() {
  // Only a computer running CortexCode can build: its card says which app it is.
  return useQuery<Peer[]>('mesh.peers:runners', async (client) => {
    const peers = (await client.call<{ peers: Peer[] }>('mesh.peers', {}))?.peers ?? [];
    return peers.filter((peer) => peer.card?.software === 'cortexcode');
  }, { topics: ['mesh.changed'] });
}

function AddRepo({ done }: { done: (next: Selection | null) => void }) {
  const client = useClient();
  const act = useAct();
  const runners = useRunners();
  const [name, setName] = useState('');
  const [prefix, setPrefix] = useState('');
  const [url, setUrl] = useState('');
  const [main, setMain] = useState('main');
  const [runner, setRunner] = useState<{ node: string; name: string }>({ node: '', name: '' });
  const ready = Boolean(name.trim() && url.trim() && main.trim());
  const add = () => {
    if (!ready) return;
    void act.run('add', 'Adding the repository', async () => {
      const created = await client.call<Repo>('manage.repo.create', {
        name: name.trim(),
        prefix: prefix.trim().toUpperCase(),
        source: { kind: 'git', url: url.trim(), main: main.trim(), changes: 'none' },
        runner: runner.node,
        runner_name: runner.name,
        managed: true,
      });
      done({ view: 'charter', repo: created.id, request: null, package: null });
    });
  };
  const items = [
    { key: '', label: 'This computer', selected: runner.node === '', onSelect: () => setRunner({ node: '', name: '' }) },
    ...(runners.data ?? []).map((peer) => ({
      key: peer.member.node,
      label: peer.member.name,
      note: peer.reachable ? 'CortexCode' : 'not reachable now',
      selected: runner.node === peer.member.node,
      onSelect: () => setRunner({ node: peer.member.node, name: peer.member.name }),
    })),
  ];
  return (
    <div className="cm-side-form cm-form" data-form="repo">
      <span className="ap">Add repo</span>
      <label className="field"><input value={name} placeholder="Name" aria-label="Name" onChange={(event) => setName(event.target.value)} /></label>
      <label className="field"><input value={prefix} placeholder="Prefix, e.g. CT" aria-label="Prefix of request numbers" maxLength={3} onChange={(event) => setPrefix(event.target.value)} /></label>
      <label className="field"><input value={url} placeholder="Git URL" aria-label="Git URL" onChange={(event) => setUrl(event.target.value)} /></label>
      <label className="field"><input value={main} placeholder="Main branch" aria-label="Main branch" onChange={(event) => setMain(event.target.value)} /></label>
      <div className="cm-form-row">
        <Menu label="Builds on" value={runner.name || 'this computer'} items={items} />
      </div>
      {runners.error && !runners.data && (
        <p className="cm-line flush">The other computers cannot be listed ({runners.errorCode === 'unavailable' ? 'the mesh is not running' : runners.error}), so only this computer is offered.</p>
      )}
      <div className="cm-form-row">
        <span className="grow" />
        <button type="button" className="detent" onClick={() => done(null)}>Cancel</button>
        <button type="button" className="detent send" aria-pressed={ready} disabled={!ready || act.busy !== null} onClick={add}>Add</button>
      </div>
      {act.failed('add') && <p className="inline-error">{act.failed('add')}</p>}
    </div>
  );
}

function Sidebar() {
  const repos = useRepos();
  const [selection, setSelection] = useManageSelection();
  const [form, setForm] = useState<'request' | 'repo' | null>(null);
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
        <button type="button" className="detent" aria-pressed={form === 'request'} title="Describe a change to one repository; it becomes a session under that repository" onClick={() => setForm(form === 'request' ? null : 'request')}>New request</button>
        <button type="button" className="detent" aria-pressed={form === 'repo'} title="Add a git repository for CortexManage to manage" onClick={() => setForm(form === 'repo' ? null : 'repo')}>Add repo</button>
      </div>
      {form === 'request' && <NewRequest repos={list} selection={selection} done={done} />}
      {form === 'repo' && <AddRepo done={done} />}
      <div className="index-h"><Cross /><span className="ap">Repositories</span></div>
      <div className="cm-side-scroll">
        {repos.error && !repos.data && <p className="inline-error">{loadFailure('The repositories', repos)}</p>}
        {repos.data && list.length === 0 && <p className="cm-line flush">No repository yet. Add one with Add repo.</p>}
        {list.map((repo, index) => <RepoBlock key={repo.id} repo={repo} index={index} selection={selection} open={setSelection} now={now} />)}
        <div className="index-h cm-side-shared"><Cross /><span className="ap">Shared by every app</span></div>
        <StandardRow selection={selection} open={setSelection} />
      </div>
    </div>
  );
}

export default definePlugin({ slot: 'workbench.sidebar@1', component: local(Sidebar) });
