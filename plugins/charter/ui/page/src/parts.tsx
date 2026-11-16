/** Small pieces the Charter view's panels share. */

import { errorMessage } from '@cortex/sdk';
import { useCallback, useState } from 'react';
import { STANDARD } from './types';
import type { Rule } from './types';

/** The theme's segmented choice (`.seg-set`): one of a few values. */
export function Seg<T extends string>({ value, options, onChange, label, disabled }: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <div className="seg-set" role="group" aria-label={label}>
      {options.map((option) => (
        <button key={option.value} type="button" aria-pressed={value === option.value} disabled={disabled} onClick={() => { if (value !== option.value) onChange(option.value); }}>
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** One call at a time, with what it said when it failed. */
export function useAct() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const run = useCallback(async <T,>(work: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError('');
    try {
      return await work();
    } catch (failure) {
      setError(errorMessage(failure));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, run, clear: useCallback(() => setError(''), []) };
}

/** A rule's id as shown beside another charter's rules: the standard's are `S-L1`. */
export function shownId(rule: Rule, repo: string): string {
  return rule.scope === STANDARD && repo !== STANDARD ? `S-${rule.id}` : rule.id;
}

/** How a rule is kept, in a word; a law or constraint kept only by text is marked. */
export function Kept({ rule }: { rule: Rule }) {
  if (rule.kept_by === 'check') return <span className="cx-ch-v" title={rule.check ? `The check ${rule.check}` : undefined}>Check{rule.check ? ` · ${rule.check}` : ''}</span>;
  if (rule.kept_by === 'validator') return <span className="cx-ch-v" title="A question on every plan">Validator</span>;
  const marked = rule.level !== 'convention';
  return <span className={marked ? 'cx-ch-v cut' : 'cx-ch-v'} title={marked ? 'Only the model reading it keeps this rule' : undefined}>Text only</span>;
}

/** One line per entry, empty lines dropped. */
export function lines(text: string): string[] {
  return text.split('\n').map((line) => line.trim()).filter(Boolean);
}
