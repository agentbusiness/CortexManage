/**
 * Add a repository, a sheet with three ways, each chosen by the person:
 * from their own GitHub repositories (vcs.github.repos; what is typed only filters the list, and
 * the one pressed is the one added, with the address and branch GitHub gives for it), new on GitHub
 * (vcs.github.repo.create, by the name typed), or any git address with a name typed beside it.
 * Nothing is worked out from the shape of what is typed. Its jobs go to any free
 * CortexCode; a computer can be named for it later in Settings.
 */

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { errorMessage, setShared, useClient, useQuery, useSlotInfo } from '@cortex/sdk';
import type { Repo, Selection } from './manage';

interface GithubStatus { connected: boolean; login?: string | null }
interface GithubRepo { full_name: string; private: boolean; description: string; fork: boolean; clone_url: string; default_branch: string }

type Way = 'github' | 'new' | 'address';

export function AddRepo({ onClose, onAdded }: { onClose: () => void; onAdded: (next: Selection) => void }) {
  const client = useClient();
  const owner = useSlotInfo();
  const status = useQuery<GithubStatus>('vcs.github.status', (c) => c.call('vcs.github.status', {}), { topics: ['plugin.swapped'] });
  const connected = status.data?.connected === true;
  const repos = useQuery<GithubRepo[]>(connected ? `vcs.github.repos:${status.data?.login ?? ''}` : null,
    async (c) => (await c.call<{ repos: GithubRepo[] }>('vcs.github.repos', {}))?.repos ?? []);
  const [way, setWay] = useState<Way>('github');
  const [filter, setFilter] = useState('');
  const [picked, setPicked] = useState<GithubRepo | null>(null);
  const [name, setName] = useState('');
  const [address, setAddress] = useState('');
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

  const wanted = filter.trim().toLowerCase();
  const listed = (repos.data ?? []).filter((r) => wanted === '' || r.full_name.toLowerCase().includes(wanted));

  const add = async (source: { name: string; url: string; main: string }) => {
    const created = await client.call<Repo>('manage.repo.create', {
      name: source.name,
      prefix: '',
      source: { kind: 'git', url: source.url, main: source.main, changes: 'none' },
      // No computer is named: its jobs go to any free CortexCode.
      runner: '',
      runner_name: '',
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
  // `full_name` is `owner/name` by GitHub's own definition: the repository's name is its second part.
  const ownName = (r: GithubRepo) => r.full_name.split('/')[1] ?? r.full_name;
  const ready = way === 'github' ? picked !== null : way === 'new' ? name.trim() !== '' : name.trim() !== '' && address.trim() !== '';
  const submit = () => {
    if (!ready) return;
    if (way === 'github' && picked) void run('Adding', () => add({ name: ownName(picked), url: picked.clone_url, main: picked.default_branch || 'main' }));
    if (way === 'new') {
      void run('Making it on GitHub', async () => {
        const made = await client.call<GithubRepo>('vcs.github.repo.create', { name: name.trim(), private: true });
        await add({ name: name.trim(), url: made.clone_url, main: made.default_branch || 'main' });
      });
    }
    if (way === 'address') void run('Adding', () => add({ name: name.trim(), url: address.trim(), main: 'main' }));
  };
  const connect = (
    <p className="hint" data-github="not-connected">
      GitHub is not connected. Connect it in{' '}
      <button type="button" className="cx-file-link" onClick={() => { onClose(); setShared('settings.open', 'github'); }}>Settings &gt; GitHub</button>
      , or add the repository by its git address.
    </p>
  );

  const root = document.getElementById('cc') ?? document.body;
  return createPortal(
    <div className="sheet-scrim open" data-slot={owner.slot} data-plugin-id={owner.pluginId} data-portal="add-repo">
      <div ref={dialog} className="sheet cx-np" role="dialog" aria-modal="true" aria-label="Add a repository">
        <form onSubmit={(event) => { event.preventDefault(); submit(); }}>
          <div className="sheet-h">
            <span className="ap">Add a repository</span>
            <span className="grow" />
            <button className="detent" type="button" onClick={onClose}>Close</button>
          </div>
          <div className="sheet-b">
            <div className="fld">
              <span className="seg-set" role="group" aria-label="Which repository">
                <button type="button" aria-pressed={way === 'github'} onClick={() => setWay('github')}>From GitHub</button>
                <button type="button" aria-pressed={way === 'new'} onClick={() => setWay('new')}>New on GitHub</button>
                <button type="button" aria-pressed={way === 'address'} onClick={() => setWay('address')}>Git address</button>
              </span>
            </div>
            {way === 'github' && (connected ? (
              <>
                <label className="fld">
                  <span className="ap faint">Your repositories</span>
                  <input type="text" value={filter} placeholder="Type to filter" spellCheck={false} autoComplete="off" data-field="filter" onChange={(event) => setFilter(event.target.value)} />
                </label>
                {repos.data ? (
                  listed.length ? (
                    <ul className="cx-np-repos" role="listbox" aria-label="Your repositories">
                      {listed.map((r) => (
                        <li key={r.full_name}>
                          <button type="button" role="option" aria-selected={picked?.full_name === r.full_name} data-repo={r.full_name} onClick={() => setPicked(r)}>
                            <span className="t">{r.full_name}</span>
                            {r.private && <span className="ap faint">private</span>}
                            {r.fork && <span className="ap faint">fork</span>}
                            {r.description && <span className="d">{r.description}</span>}
                          </button>
                        </li>
                      ))}
                    </ul>
                  ) : <p className="hint">None of your repositories has that in its name.</p>
                ) : repos.error ? <p className="inline-error">GitHub did not list your repositories: {repos.error}</p>
                  : <p className="hint">Reading your repositories…</p>}
              </>
            ) : status.data ? connect : status.error ? <p className="hint">The GitHub connection is not available ({status.error}). Add the repository by its git address.</p> : null)}
            {way === 'new' && (connected ? (
              <label className="fld">
                <span className="ap faint">Name of the new repository</span>
                <input type="text" value={name} placeholder="Name" spellCheck={false} autoComplete="off" data-field="name" onChange={(event) => setName(event.target.value)} />
              </label>
            ) : status.data ? connect : null)}
            {way === 'address' && (
              <>
                <label className="fld">
                  <span className="ap faint">Git address</span>
                  <input type="text" value={address} placeholder="https://… or git@…" spellCheck={false} autoComplete="off" data-field="address" onChange={(event) => setAddress(event.target.value)} />
                </label>
                <label className="fld">
                  <span className="ap faint">Name</span>
                  <input type="text" value={name} placeholder="What to call it here" spellCheck={false} autoComplete="off" data-field="name" onChange={(event) => setName(event.target.value)} />
                </label>
              </>
            )}
            {error && <p className="inline-error" role="alert">{error}</p>}
          </div>
          <div className="sheet-f">
            <span className="grow" />
            <button className="detent" type="button" onClick={onClose}>Cancel</button>
            <button className="detent" type="submit" data-action="add-repo" disabled={!ready || busy !== null}>
              {busy ? `${busy}…` : way === 'new' ? 'Make it and add it' : 'Add'}
            </button>
          </div>
        </form>
      </div>
    </div>,
    root,
  );
}
