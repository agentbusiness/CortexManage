/**
 * Add a repository, a sheet: picked from the person's own GitHub repositories (vcs.github.repos,
 * filtered by what is typed), typed as owner/name or as any git address, or made new on GitHub
 * (vcs.github.repo.create). Nothing else is asked: the name comes from the repository, the branch
 * is its default branch, and it builds on the CortexCode that is reachable. All of it can be
 * changed later in Settings.
 */

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { errorMessage, setShared, useClient, useQuery, useSlotInfo } from '@cortex/sdk';
import type { Repo, Selection } from './manage';

interface GithubStatus { connected: boolean; login?: string | null }
interface GithubRepo { full_name: string; private: boolean; description: string; fork: boolean; clone_url: string; default_branch: string }
interface Peer { member: { node: string; name: string }; card?: { software?: string } | null; reachable: boolean }

/** The name a repository's address suggests: its last part, without `.git`. */
function nameOf(repo: string): string {
  const last = repo.trim().replace(/\/+$/, '').split(/[/:]/).pop() ?? '';
  return last.replace(/\.git$/, '');
}

/** The git address of what was typed: an address as it is, `owner/name` on GitHub. */
function addressOf(typed: string): string {
  const text = typed.trim();
  if (/^[\w.-]+\/[\w.-]+$/.test(text)) return `https://github.com/${text.replace(/\.git$/, '')}.git`;
  return text;
}

export function AddRepo({ onClose, onAdded }: { onClose: () => void; onAdded: (next: Selection) => void }) {
  const client = useClient();
  const owner = useSlotInfo();
  const status = useQuery<GithubStatus>('vcs.github.status', (c) => c.call('vcs.github.status', {}), { topics: ['plugin.swapped'] });
  const connected = status.data?.connected === true;
  const repos = useQuery<GithubRepo[]>(connected ? `vcs.github.repos:${status.data?.login ?? ''}` : null,
    async (c) => (await c.call<{ repos: GithubRepo[] }>('vcs.github.repos', {}))?.repos ?? []);
  // Only a computer running CortexCode can build: its card says which app it is.
  const runners = useQuery<Peer[]>('mesh.peers:runners', async (c) => ((await c.call<{ peers: Peer[] }>('mesh.peers', {}))?.peers ?? []).filter((peer) => peer.card?.software === 'cortexcode'), { topics: ['mesh.changed'] });
  const [repo, setRepo] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // The dialog pattern: focus moves in, Escape closes, focus returns where it was.
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.querySelector('input')?.focus({ preventScroll: true });
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      closeRef.current();
    };
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', onKey);
      if (before?.isConnected) before.focus({ preventScroll: true });
    };
  }, []);

  const typed = repo.trim();
  const listed = (repos.data ?? []).filter((r) => !typed || r.full_name.toLowerCase().includes(typed.toLowerCase()));
  const picked = (repos.data ?? []).find((r) => r.full_name === typed) ?? null;
  // A bare name that is none of the person's repositories: it can be made new on GitHub.
  const fresh = connected && /^[\w.-]+$/.test(typed) && !(repos.data ?? []).some((r) => nameOf(r.full_name).toLowerCase() === typed.toLowerCase());
  const name = nameOf(typed);

  const add = async (source: { url: string; main: string }) => {
    const runner = (runners.data ?? []).find((peer) => peer.reachable) ?? null;
    const created = await client.call<Repo>('manage.repo.create', {
      name,
      prefix: '',
      source: { kind: 'git', url: source.url, main: source.main, changes: 'none' },
      runner: runner?.member.node ?? '',
      runner_name: runner?.member.name ?? '',
      managed: true,
    });
    onAdded({ view: 'new', repo: created.id, request: null, package: null });
    onClose();
  };
  const run = async (step: string, work: () => Promise<void>) => {
    setError(null);
    setBusy(step);
    try {
      await work();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };
  const addTyped = () => run('Adding', () => add(picked ? { url: picked.clone_url, main: picked.default_branch || 'main' } : { url: addressOf(typed), main: 'main' }));
  const makeNew = () => run('Making it on GitHub', async () => {
    const made = await client.call<GithubRepo>('vcs.github.repo.create', { name: typed, private: true });
    await add({ url: made.clone_url, main: made.default_branch || 'main' });
  });

  const root = document.getElementById('cc') ?? document.body;
  return createPortal(
    <div className="sheet-scrim open" data-slot={owner.slot} data-plugin-id={owner.pluginId} data-portal="add-repo">
      <div ref={dialog} className="sheet cx-np" role="dialog" aria-modal="true" aria-label="Add a repository">
        <form onSubmit={(event) => { event.preventDefault(); if (fresh) void makeNew(); else void addTyped(); }}>
          <div className="sheet-h">
            <span className="ap">Add a repository</span>
            <span className="grow" />
            <button className="detent" type="button" onClick={onClose}>Close</button>
          </div>
          <div className="sheet-b">
            <label className="fld">
              <span className="ap faint">Repository</span>
              <input type="text" value={repo} placeholder={connected ? 'Search yours, or type owner/name, a git address, or a new name' : 'owner/name or a git address'} spellCheck={false} autoComplete="off" data-field="repo"
                onChange={(event) => setRepo(event.target.value)} />
            </label>
            {connected ? (
              repos.data ? (
                listed.length ? (
                  <ul className="cx-np-repos" role="listbox" aria-label="Your repositories">
                    {listed.map((r) => (
                      <li key={r.full_name}>
                        <button type="button" role="option" aria-selected={r.full_name === typed} data-repo={r.full_name} onClick={() => setRepo(r.full_name)}>
                          <span className="t">{r.full_name}</span>
                          {r.private && <span className="ap faint">private</span>}
                          {r.fork && <span className="ap faint">fork</span>}
                          {r.description && <span className="d">{r.description}</span>}
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : <p className="hint">{fresh ? `None of your repositories is called ${typed}. Make it new on GitHub, as a private repository.` : 'None of your repositories match; Add takes what is typed.'}</p>
              ) : repos.error ? <p className="inline-error">GitHub did not list your repositories: {repos.error}</p>
                : <p className="hint">Reading your repositories…</p>
            ) : status.data ? (
              <p className="hint" data-github="not-connected">
                GitHub is not connected, so your repositories are not listed. Connect it in{' '}
                <button type="button" className="cx-file-link" onClick={() => { onClose(); setShared('settings.open', 'github'); }}>Settings &gt; GitHub</button>.
                A repository can still be typed as owner/name or as a git address.
              </p>
            ) : status.error ? (
              <p className="hint">The GitHub connection is not available ({status.error}). A repository can be typed as owner/name or as a git address.</p>
            ) : null}
            {error && <p className="inline-error" role="alert">{error}</p>}
          </div>
          <div className="sheet-f">
            <span className="grow" />
            <button className="detent" type="button" onClick={onClose}>Cancel</button>
            <button className="detent" type="submit" data-action="add-repo" disabled={!name || busy !== null}>
              {busy ? `${busy}…` : fresh ? `Make ${typed} on GitHub and add it` : 'Add'}
            </button>
          </div>
        </form>
      </div>
    </div>,
    root,
  );
}
