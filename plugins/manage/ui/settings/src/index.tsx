/**
 * settings.manage@1 — the rules every repository's commits and releases keep, as the person
 * writes them (`manage.rules.get` / `manage.rules.edit`). A repository's own rules are in its
 * settings; both are given to the model that writes commit messages and to the check before a
 * change lands.
 */

import { useState } from 'react';
import { definePlugin, errorMessage, useClient, useQuery } from '@cortex/sdk';
import './settings.css';

function RulesPanel() {
  const client = useClient();
  const rules = useQuery<string>('manage.rules.get', async (c) => (await c.call<{ rules: string }>('manage.rules.get', {}))?.rules ?? '', { topics: ['manage.changed'] });
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const text = draft ?? rules.data ?? '';
  const dirty = draft !== null && draft.trim() !== (rules.data ?? '').trim();
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await client.call('manage.rules.edit', { rules: text });
      await rules.refresh();
      setDraft(null);
      setSaved(true);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="cx-settings-panel-body" data-build={__CORTEX_BUILD__} data-panel="rules">
      <h3 className="cx-settings-title">Rules for every repository</h3>
      <textarea
          className="cm-text"
          rows={8}
          value={text}
          aria-label="Rules for every repository's commits and releases"
          placeholder="For example: never put personal data in code, commits or release notes."
          onChange={(event) => {
            setSaved(false);
            setDraft(event.target.value);
          }}
        />
      <div className="cm-rules-foot">
        {saved && !dirty && <span className="ap faint">Saved</span>}
        <span className="grow" />
        <button className="detent send" type="button" disabled={!dirty || busy} onClick={() => void save()}>{busy ? 'Saving…' : 'Save'}</button>
      </div>
      {(error || (rules.error && !rules.data)) && <p className="inline-error" role="alert">{error ?? rules.error}</p>}
    </div>
  );
}

export default definePlugin({ slot: 'settings.manage@1', component: RulesPanel });
