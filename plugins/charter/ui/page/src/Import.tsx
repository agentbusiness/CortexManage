/** The start of a charter: the repository's AGENTS.md pasted in, split into a draft's rules. */

import { useClient } from '@cortex/sdk';
import { useState } from 'react';
import { useAct } from './parts';

export function ImportPanel({ repo, replaces, onDone }: { repo: string; replaces?: boolean; onDone?: () => void }) {
  const client = useClient();
  const act = useAct();
  const [file, setFile] = useState('AGENTS.md');
  const [text, setText] = useState('');

  const make = () => act.run(async () => {
    await client.call('charter.import', { repo, file: file.trim() || 'AGENTS.md', text });
    setText('');
    onDone?.();
  });
  const empty = () => act.run(async () => {
    // A charter written from nothing: the draft starts with its identity, the rules follow.
    await client.call('charter.rule.edit', { repo, identity: { what: '', who_for: '', never: [] } });
    onDone?.();
  });

  return (
    <div className="cx-ch-form">
      <p className="cx-ch-note">
        {replaces
          ? 'Importing again replaces the whole draft with the rules of the text below.'
          : 'Paste the file’s text. Each paragraph and each list item becomes one rule of a draft, linked to its lines. Every rule starts as a convention; you set the laws and constraints in the draft.'}
      </p>
      <label className="cx-ch-label">
        <span className="ap faint">File name</span>
        <span className="field"><input type="text" value={file} spellCheck={false} onChange={(event) => setFile(event.target.value)} /></span>
      </label>
      <label className="cx-ch-label">
        <span className="ap faint">The file&rsquo;s text</span>
        <textarea className="cx-ch-area mono" rows={14} value={text} spellCheck={false} placeholder="Paste AGENTS.md here" onChange={(event) => setText(event.target.value)} />
      </label>
      {act.error && <p className="inline-error">{act.error}</p>}
      <div className="cx-ch-acts">
        <button type="button" className="detent" disabled={act.busy || !text.trim()} onClick={() => void make()}>{replaces ? 'Import again' : 'Make a draft'}</button>
        {!replaces && <button type="button" className="detent" disabled={act.busy} onClick={() => void empty()}>Start empty</button>}
      </div>
    </div>
  );
}
