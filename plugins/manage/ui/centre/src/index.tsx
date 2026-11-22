/**
 * workbench.chat@1 — the centre column. The one selection says what is open: a request's session
 * (hosted: `manage.session@1`), a repository's pages (Charter and Releases hosted, Runner its
 * own), or Waits on you (its own). A hosted page with no package
 * installed is one plain line saying which part is missing.
 */

import { useState } from 'react';
import { Menu, definePlugin, useClient, useHost, useNow, useQuery, useUiPlugins } from '@cortex/sdk';
import { CHANGED, loadFailure, local, sessionOf, useManageSelection, type Repo, type RequestView, type Selection, type View } from './manage';
import { RunnerPage } from './Runner';
import { SchedulePage } from './Schedule';
import './manage.css';
import './centre.css';
import './composer.css';
import { Composer } from './Composer';
import type { AttachContext, AttachShell, Attachment, PutParams, PutResult } from './attachments';

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
  const host = useHost();
  const managed = repos.filter((repo) => repo.managed);
  const repo = managed.find((candidate) => candidate.id === selection.repo) ?? managed[0] ?? null;
  const [draft, setDraft] = useState('');
  const [files, setFiles] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const send = async (text: string, attachments: Attachment[]): Promise<boolean> => {
    if (!repo) return false;
    setError(null);
    try {
      const created = await client.call<RequestView>('manage.request.create', { repo: repo.id, text, from: { kind: 'person' }, ...(attachments.length > 0 ? { attachments } : {}) });
      open(sessionOf(created));
      return true;
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
      return false;
    }
  };
  const attach: AttachContext = {
    put: (params: PutParams) => client.call<PutResult>('chatflow-manage.attachment.put', params),
    shell: host.shell as unknown as AttachShell | null,
    roots: [],
    projectId: null,
    sessionId: null,
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
      <div className="cm-new-body" />
      <div className="composer cm-composer">
        {error && <p className="inline-error" role="alert">{error}</p>}
        <Composer draft={draft} onDraft={setDraft} onSend={send} disabled={false} autoFocus
          placeholder={`What should change in ${repo.name}?`} attachments={files} onAttachments={setFiles} attach={attach} />
      </div>
    </div>
  );
}

function Centre() {
  const [selection, setSelection] = useManageSelection();
  const repos = useQuery<Repo[]>('manage.repo.list', async (client) => (await client.call<{ repos: Repo[] }>('manage.repo.list', {}))?.repos ?? [], { topics: [CHANGED] });
  const repo = repos.data?.find((candidate) => candidate.id === selection.repo);
  const now = useNow(60_000, true);
  let body;
  switch (selection.view) {
    case 'session':
      body = selection.request
        ? <Hosted slot="manage.session@1" missing="The session view is missing: the chat flow part (chatflow-manage) is not installed, so a request's thread cannot be shown." />
        : <p className="cm-line">Choose a request on the board, or press New request.</p>;
      break;
    case 'schedule':
      body = <SchedulePage repos={repos.data ?? []} open={setSelection} now={now} />;
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
    default:
      body = <p className="cm-line">Choose a request on the board, or press New request.</p>;
  }
  return <div className="cm-centre" data-view={selection.view} data-build={__CORTEX_BUILD__}>{body}</div>;
}

export default definePlugin({ slot: 'workbench.chat@1', component: local(Centre) });
