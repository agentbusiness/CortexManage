/**
 * page.standard@1: the Standard view. One row per part of the mesh standard, one column per
 * member; each cell says how that member's copy stands against the lock. A cell that differs
 * lists its files when chosen. Above: the lock's version and when the files were last checked,
 * Check now, Write lock from a member, Ask the members (the handshake). Below: the members, with
 * each one's folder on this computer and whether it is left out for now.
 */

import { Menu, ago, definePlugin, errorMessage, useClient, useNow, useQuery } from '@cortex/sdk';
import { useEffect, useState } from 'react';
import './standard.css';

interface Member {
  id: string;
  name: string;
  priority: number;
  folders: Record<string, string>;
  without?: string[];
  deferred?: boolean;
  root?: string;
  names?: string[];
}
interface Cell { member: string; part: string; state: string; files?: string[]; count?: number }
interface Report { version: string; parts: string[]; members: Member[]; cells: Cell[]; drift: string[]; checked_ms: number }
interface HandshakeRow { node: string; name: string; software: string; state: string; detail?: string }
interface Handshake { rows: HandshakeRow[]; ms: number }

/** What a cell reads, and whether it is a difference (`bad`) or nothing to act on (`ok`). */
function reading(cell: Cell | undefined): { text: string; tone: 'ok' | 'bad' } {
  switch (cell?.state) {
    case 'same': return { text: '=', tone: 'ok' };
    case 'names': return { text: '≈ name', tone: 'ok' };
    case 'differs': return { text: `differs (${cell.count ?? cell.files?.length ?? 0})`, tone: 'bad' };
    case 'missing': return { text: 'missing', tone: 'bad' };
    case 'without': return { text: 'without', tone: 'ok' };
    case 'deferred': return { text: 'deferred', tone: 'ok' };
    case 'not_here': return { text: 'not here', tone: 'ok' };
    default: return { text: '', tone: 'ok' };
  }
}

/** One member's row: its folder on this computer and whether it is left out for now. */
function MemberRow({ member, busy, onSave }: { member: Member; busy: boolean; onSave: (next: Member) => void }) {
  const kept = member.root ?? '';
  const [root, setRoot] = useState(kept);
  // A change made elsewhere (another window, the first start) shows here too.
  useEffect(() => setRoot(kept), [kept]);
  const changed = root.trim() !== kept;
  const save = () => {
    if (changed && !busy) onSave({ ...member, root: root.trim() });
  };
  return (
    <div className="std-member">
      <span className="std-who">
        <span className="ap">{member.name}</span>
        <span className="ap faint num">{member.id} · {member.priority}</span>
      </span>
      <label className="field std-root">
        <input
          value={root}
          placeholder="Folder on this computer"
          spellCheck={false}
          aria-label={`${member.name}: folder on this computer`}
          onChange={(event) => setRoot(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') save();
            if (event.key === 'Escape') setRoot(kept);
          }}
        />
      </label>
      <button type="button" className="detent" disabled={!changed || busy} onClick={save}>Save</button>
      <span className="std-defer">
        <span className="ap faint">Deferred</span>
        <button
          type="button"
          className="sw"
          aria-pressed={!!member.deferred}
          aria-label={`${member.name}: left out for now`}
          disabled={busy}
          onClick={() => onSave({ ...member, deferred: !member.deferred })}
        />
      </span>
    </div>
  );
}

function StandardPage() {
  const client = useClient();
  const now = useNow(30_000, true);
  const report = useQuery<Report>('standard.get', (c) => c.call<Report>('standard.get', {}), { topics: ['standard.changed'] });
  const [busy, setBusy] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [chosen, setChosen] = useState<{ member: string; part: string } | null>(null);
  const [handshake, setHandshake] = useState<(Handshake & { at: number }) | null>(null);
  const [lockText, setLockText] = useState<string | null>(null);

  const data = report.data;
  const members = data?.members ?? [];
  const parts = data?.parts ?? [];
  const cellOf = (member: string, part: string) => data?.cells.find((cell) => cell.member === member && cell.part === part);

  /** One call at a time; what went wrong is said in the page. */
  const run = async (what: string, work: () => Promise<void>) => {
    if (busy) return;
    setBusy(what);
    setProblem(null);
    try {
      await work();
      await report.refresh();
    } catch (error) {
      setProblem(errorMessage(error));
    } finally {
      setBusy(null);
    }
  };

  const check = () => run('check', async () => { await client.call('standard.check', {}); });
  const writeLock = (from: string) => run('lock', async () => {
    await client.call('standard.lock.write', { from });
    setLockText(null);
  });
  const ask = () => run('handshake', async () => {
    const answer = await client.call<Handshake>('standard.handshake', {});
    setHandshake({ ...answer, at: Date.now() });
  });
  const saveMember = (next: Member) => run('member', async () => { await client.call('standard.member.edit', next); });
  const showLock = () => {
    if (lockText !== null) {
      setLockText(null);
      return;
    }
    void run('print', async () => {
      const answer = await client.call<{ text: string }>('standard.lock.print', {});
      setLockText(answer.text);
    });
  };

  const version = data?.version ?? '';
  const open = chosen ? cellOf(chosen.member, chosen.part) : undefined;
  const openMember = chosen ? members.find((member) => member.id === chosen.member) : undefined;
  // Only a member with a folder here can be the one the lock is made from.
  const sources = members.filter((member) => (member.root ?? '') !== '');
  /** The computers of one member that were asked, by the app each said it is. */
  const answersOf = (member: Member) => (handshake?.rows ?? []).filter((row) => row.software === member.id);
  const strangers = (handshake?.rows ?? []).filter((row) => !members.some((member) => member.id === row.software));

  return (
    <div className="std-page">
      <div className="colhead">
        <span className="ap">Mesh standard{version ? ` · v${version}` : ''}</span>
        <span className="grow" />
        <span className="ap faint">
          {data && data.checked_ms > 0 ? `files ${ago(data.checked_ms, now)}` : 'files not checked yet'}
          {handshake ? ` · handshake ${ago(handshake.at, now)}` : ''}
        </span>
        <button type="button" className="detent" disabled={!!busy || !version} onClick={check}>
          {busy === 'check' ? 'Checking' : 'Check now'}
        </button>
        <Menu
          variant="detent"
          label={busy === 'lock' ? 'Writing lock' : 'Write lock from'}
          disabled={!!busy || sources.length === 0}
          title="Make the lock from one member's copies. Every other member is then compared with them."
          items={sources.map((member) => ({
            key: member.id,
            label: member.name,
            note: member.root,
            selected: false,
            onSelect: () => { void writeLock(member.id); },
          }))}
        />
      </div>

      <div className="std-body">
        {problem && <p className="std-problem" role="alert">{problem}</p>}
        {report.error && !data && <p className="std-problem" role="alert">{report.error}</p>}

        {data && !version && (
          <p className="std-note">
            There is no lock yet. Choose Write lock from and the member whose copies are the standard; every other member is then compared with them.
          </p>
        )}

        {members.length > 0 && (
          <div className="std-tw">
            <table className="std-drift">
              <thead>
                <tr>
                  <th>Part</th>
                  {members.map((member) => <th key={member.id} title={`${member.id} · priority ${member.priority}`}>{member.name}</th>)}
                </tr>
              </thead>
              <tbody>
                {parts.map((part) => (
                  <tr key={part}>
                    <td className="k">{part}</td>
                    {members.map((member) => {
                      const cell = cellOf(member.id, part);
                      const { text, tone } = reading(cell);
                      const files = cell?.files ?? [];
                      const on = chosen?.member === member.id && chosen.part === part;
                      return (
                        <td key={member.id} className={tone}>
                          {files.length > 0 ? (
                            <button
                              type="button"
                              className="std-cell"
                              aria-pressed={on}
                              onClick={() => setChosen(on ? null : { member: member.id, part })}
                            >
                              {text}
                            </button>
                          ) : text}
                        </td>
                      );
                    })}
                  </tr>
                ))}
                {handshake && (
                  <tr>
                    <td className="k">Handshake</td>
                    {members.map((member) => {
                      const rows = answersOf(member);
                      if (rows.length === 0) return <td key={member.id} className="ok">not on the mesh</td>;
                      return (
                        <td key={member.id}>
                          {rows.map((row) => (
                            <span key={row.node} className={`std-answer ${row.state === 'fails' ? 'bad' : 'ok'}`} title={row.detail}>
                              {row.state}{rows.length > 1 ? ` · ${row.name}` : ''}
                            </span>
                          ))}
                        </td>
                      );
                    })}
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}

        {open && openMember && (open.files?.length ?? 0) > 0 && (
          <div className="std-block">
            <div className="hd">
              <span className="pip" />
              <span className="ap">{openMember.name} · {open.part} · {reading(open).text}</span>
            </div>
            <ul className="std-files">
              {(open.files ?? []).map((file) => <li key={file}>{openMember.folders[open.part] ? `${openMember.folders[open.part]}/` : ''}{file}</li>)}
            </ul>
            {(open.count ?? 0) > (open.files?.length ?? 0) && (
              <p className="std-sub">and {(open.count ?? 0) - (open.files?.length ?? 0)} more.</p>
            )}
          </div>
        )}

        {handshake && (handshake.rows.some((row) => row.state !== 'answers') || strangers.length > 0) && (
          <div className="std-block">
            <div className="hd"><span className="pip hollow" /><span className="ap">Handshake</span></div>
            <ul className="std-files">
              {handshake.rows.filter((row) => row.state !== 'answers' || strangers.includes(row)).map((row) => (
                <li key={row.node}>{row.name || row.node.slice(0, 8)}{row.software ? ` (${row.software})` : ''}: {row.state}{row.detail ? `. ${row.detail}` : ''}</li>
              ))}
            </ul>
          </div>
        )}
        {handshake && handshake.rows.length === 0 && <p className="std-note">No other member is on the mesh now, so there was no one to ask.</p>}

        <p className="std-sub">
          = identical bytes · ≈ name: identical once the app's own names are replaced · differs: files that are not the lock's ·
          missing: the part is not in the checkout · without: left out by decision · deferred: not checked for now · not here: no folder on this computer.
        </p>

        <div className="std-acts">
          <button type="button" className="detent" disabled={!!busy} onClick={ask}>
            {busy === 'handshake' ? 'Asking' : 'Ask the members'}
          </button>
          <button type="button" className="detent" aria-pressed={lockText !== null} disabled={!!busy || !version} onClick={showLock}>
            standard.lock
          </button>
        </div>
        {lockText !== null && <pre className="std-lock">{lockText}</pre>}

        <div className="std-section">
          <span className="ap">Members</span>
          {members.map((member) => <MemberRow key={member.id} member={member} busy={!!busy} onSave={saveMember} />)}
        </div>
      </div>
    </div>
  );
}

export default definePlugin({ slot: 'page.standard@1', component: StandardPage });
