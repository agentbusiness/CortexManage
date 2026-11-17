/**
 * workbench.chat@1 — the centre column. The one selection says what is open: a request's session
 * (hosted: `manage.session@1`), a repository's pages (Charter and Releases hosted, Runner its
 * own), the mesh standard (hosted), or Waits on you (its own). A hosted page with no package
 * installed is one plain line saying which part is missing.
 */

import { useState } from 'react';
import { Menu, definePlugin, useClient, useHost, useQuery, useUiPlugins } from '@cortex/sdk';
import { CHANGED, loadFailure, local, sessionOf, useManageSelection, type Repo, type RequestView, type Selection, type View } from './manage';
import { InboxPage } from './Inbox';
import { RunnerPage } from './Runner';
import './manage.css';
import './centre.css';

/** A slot another part fills, or the line that says the part is not there. */
function Hosted({ slot, missing }: { slot: string; missing: string }) {
  const host = useHost();
  const filled = useUiPlugins().some((plugin) => plugin.slot === slot);
  if (!filled) return <p className="cm-line" data-missing={slot}>{missing}</p>;
  return <host.Slot name={slot} />;
}

const PAGES: Array<{ view: View; label: string }> = [
  { view: 'charter', label: 'Charter' },
  { view: 'releases', label: 'Releases' },
  { view: 'runner', label: 'Runner' },
];

function RepoHead({ selection, open, repo }: { selection: Selection; open: (next: Selection) => void; repo: Repo | undefined }) {
  return (
    <div className="colhead">
      <span className="cm-ptabs" role="tablist" aria-label="Repository pages">
        {PAGES.map((page) => (
          <button key={page.view} type="button" role="tab" className="ap" aria-pressed={selection.view === page.view} aria-selected={selection.view === page.view}
            onClick={() => open({ view: page.view, repo: selection.repo, request: null, package: null })}>
            {page.label}
          </button>
        ))}
      </span>
      <span className="grow" />
      {repo && <span className="ap faint">{repo.name}{repo.managed ? '' : ' · not managed yet'}</span>}
    </div>
  );
}

/** A new request, as a conversation not yet begun: the first message makes the request and opens
 * its session, where the manager answers, rates it and plans it. */
function NewSession({ repos, selection, open }: { repos: Repo[]; selection: Selection; open: (next: Selection) => void }) {
  const client = useClient();
  const managed = repos.filter((repo) => repo.managed);
  const repo = managed.find((candidate) => candidate.id === selection.repo) ?? managed[0] ?? null;
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const send = async () => {
    const text = draft.trim();
    if (!repo || !text || busy) return;
    setBusy(true);
    setError(null);
    try {
      const created = await client.call<RequestView>('manage.request.create', { repo: repo.id, text, from: { kind: 'person' } });
      setDraft('');
      open(sessionOf(created));
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };
  if (!repo) return <p className="cm-line">Add a repository first: a request is always for one.</p>;
  return (
    <div className="cm-new">
      <div className="colhead">
        <span className="ap">New request</span>
        <span className="grow" />
        {managed.length > 1
          ? <Menu label="For" value={repo.name} items={managed.map((candidate) => ({ key: candidate.id, label: candidate.name, selected: candidate.id === repo.id, onSelect: () => open({ view: 'new', repo: candidate.id, request: null, package: null }) }))} />
          : <span className="ap faint">{repo.name}</span>}
      </div>
      <div className="cm-new-body">
        <p className="cm-line">Say what should change in {repo.name}, in your own words. The manager answers here, rates the change and plans it; you can keep talking until it is right.</p>
      </div>
      <div className="composer cm-composer">
        {error && <p className="inline-error" role="alert">{error}</p>}
        <div className="cm-box">
          <textarea className="write" rows={2} value={draft} autoFocus placeholder={`What should change in ${repo.name}?`} aria-label="Message"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void send();
              }
            }} />
          <div className="tools cm-tools" role="group" aria-label="Message tools">
            <div className="grow" />
            <button type="button" className="detent send" disabled={busy || draft.trim() === ''} onClick={() => void send()}>{busy ? 'Sending…' : 'Send'}</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Centre() {
  const [selection, setSelection] = useManageSelection();
  const repos = useQuery<Repo[]>('manage.repo.list', async (client) => (await client.call<{ repos: Repo[] }>('manage.repo.list', {}))?.repos ?? [], { topics: [CHANGED] });
  const repo = repos.data?.find((candidate) => candidate.id === selection.repo);
  let body;
  switch (selection.view) {
    case 'session':
      body = selection.request
        ? <Hosted slot="manage.session@1" missing="The session view is missing: the chat flow part (chatflow-manage) is not installed, so a request's thread cannot be shown." />
        : <p className="cm-line">Choose a request in the sidebar to open its session.</p>;
      break;
    case 'new':
      body = <NewSession repos={repos.data ?? []} selection={selection} open={setSelection} />;
      break;
    case 'charter':
    case 'releases':
    case 'runner':
      body = (
        <>
          <RepoHead selection={selection} open={setSelection} repo={repo} />
          {!selection.repo && <p className="cm-line">Choose a repository in the sidebar.</p>}
          {selection.repo && repos.error && !repos.data && <p className="inline-error">{loadFailure('The repository', repos)}</p>}
          {selection.repo && selection.view === 'charter' && <Hosted slot="page.charter@1" missing="The Charter page is missing: the charter part is not installed." />}
          {selection.repo && selection.view === 'releases' && <Hosted slot="page.releases@1" missing="The Releases page is missing: the release train part is not installed." />}
          {selection.repo && selection.view === 'runner' && <RunnerPage repoId={selection.repo} repo={repo} />}
        </>
      );
      break;
    case 'standard':
      body = <Hosted slot="page.standard@1" missing="The Mesh standard page is missing: the standard part is not installed." />;
      break;
    default:
      body = <InboxPage repos={repos.data ?? []} open={setSelection} />;
  }
  return <div className="cm-centre" data-view={selection.view} data-build={__CORTEX_BUILD__}>{body}</div>;
}

export default definePlugin({ slot: 'workbench.chat@1', component: local(Centre) });
