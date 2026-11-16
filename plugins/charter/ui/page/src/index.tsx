/**
 * page.charter@1 — the Charter view of the repository the shared selection names, or of the
 * shared standard when it names none. No charter yet: the import panel. A draft: levels set rule
 * by rule, then accepted. An accepted charter: the rules on the left, the proposal panel on the
 * right.
 */

import { ServiceProblem, definePlugin, useQuery, useShared } from '@cortex/sdk';
import { AcceptedView } from './Accepted';
import { DraftView } from './Draft';
import { ImportPanel } from './Import';
import { STANDARD } from './types';
import type { Charter, CharterRow, Printed, Selection } from './types';
import './charter.css';

const NO_SELECTION: Selection = { view: 'inbox', repo: null, request: null, package: null };

function CharterPage() {
  const [selection] = useShared<Selection>('manage.selection', NO_SELECTION);
  const repo = selection?.repo || STANDARD;
  // A repository without a charter is not an error: the list says whether there is one to get.
  const loaded = useQuery<Charter | null>(`charter:${repo}`, async (client) => {
    const listed = await client.call<{ charters: CharterRow[] }>('charter.list', {});
    if (!listed.charters.some((row) => row.repo === repo)) return null;
    return client.call<Charter>('charter.get', { repo });
  }, { topics: ['charter.changed'] });
  const charter = loaded.data ?? null;
  // The print says how far into the file the laws and constraints reach; it follows the charter.
  const printed = useQuery<Printed>(charter ? `charter.print:${repo}:${charter.digest ?? ''}:${charter.standard_version ?? ''}` : null, (client) => client.call<Printed>('charter.print', { repo }));

  const name = repo === STANDARD ? 'Mesh standard' : repo;
  const where = charter
    ? [name, charter.draft ? 'charter draft' : `charter ${charter.version}`, repo !== STANDARD && charter.standard_version ? `standard ${charter.standard_version}` : ''].filter(Boolean).join(' · ')
    : `${name} · no charter yet`;

  return (
    <div className="cx-charter" data-build={__CORTEX_BUILD__} data-repo={repo}>
      <div className="cx-ch-where"><span className="ap">Charter</span><span className="grow" /><span className="ap faint">{where}</span></div>
      {loaded.error && loaded.data === undefined && <ServiceProblem service="charter" state={loaded} className="inline-error" />}
      {loaded.data === null && (
        <div className="cx-ch-start">
          <div className="cx-ch-block">
            <div className="cx-ch-hd"><span className="pip hollow" /><span className="ap">Import AGENTS.md</span></div>
            <ImportPanel repo={repo} onDone={() => void loaded.refresh()} />
          </div>
        </div>
      )}
      {charter && charter.draft && <DraftView charter={charter} printed={printed.data} />}
      {charter && !charter.draft && <AcceptedView charter={charter} printed={printed.data} />}
    </div>
  );
}

export default definePlugin({ slot: 'page.charter@1', component: CharterPage });
