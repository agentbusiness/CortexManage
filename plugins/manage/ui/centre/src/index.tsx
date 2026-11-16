/**
 * workbench.chat@1 — the centre column. The one selection says what is open: a request's session
 * (hosted: `manage.session@1`), a repository's pages (Charter and Releases hosted, Runner its
 * own), the mesh standard (hosted), or Waits on you (its own). A hosted page with no package
 * installed is one plain line saying which part is missing.
 */

import { definePlugin, useHost, useQuery, useUiPlugins } from '@cortex/sdk';
import { CHANGED, loadFailure, local, useManageSelection, type Repo, type Selection, type View } from './manage';
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
