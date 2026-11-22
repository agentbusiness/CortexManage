/**
 * Error flow and Metrics flow: what the apps of the managed repositories reported about themselves
 * (`manage.reports.read`), kept a week. One stack, the same for both: the kept days as the only
 * navigation, the figures, when it happened by the minute, the filters, and the rows themselves.
 * Every name is a filter; pick it again to drop it. The board's bar chooses the repository and the
 * computer. Metrics flow shows the model calls (every one: model, tag, the computer that served it,
 * tokens) or the methods the apps served.
 */

import { Fragment, useEffect, useMemo, useState, type ReactNode } from 'react';
import { errorMessage, useClient, useQuery, type Client } from '@cortex/sdk';
import { CHANGED, loadFailure } from './manage';

export type Table = 'errors' | 'calls' | 'models';

interface Day {
  date: string;
  count: number;
  groups: number;
  by_kind: Record<string, number>;
}

export interface Computer {
  node: string;
  name: string;
}

interface Answer {
  table: string;
  today: string;
  keep_days: number;
  days: Day[];
  computers: Computer[];
  rows: Row[];
  /** Each failure's request, by `repo|fingerprint`: the open one, else the newest. */
  requests?: Record<string, RequestLink>;
}

/** The request a reported failure became. */
export interface RequestLink {
  id: string;
  repo: string;
  package: string;
  open: boolean;
  /** Its stage while open, how it ended otherwise. */
  state: string;
}

/** A row as the app recorded it, with who sent it beside it. */
type Row = Record<string, unknown> & {
  count: number;
  first_at_ms: number;
  last_at_ms: number;
  app?: string;
  computer?: string;
  repo?: string;
  version?: string;
};

interface Span {
  from: string;
  to: string;
}

// ---- formatting ---------------------------------------------------------------------------------

const DAY_MS = 86_400_000;

const fmtInt = (n: number) => Math.round(n || 0).toLocaleString('en').replace(/,/g, ' ');

function fmtUs(us: number): string {
  if (!Number.isFinite(us)) return '—';
  const ms = us / 1000;
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)} min`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`;
  if (ms >= 10) return `${Math.round(ms)} ms`;
  return `${ms.toFixed(1)} ms`;
}

function fmtTokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)} M`;
  if (n >= 1e4) return `${(n / 1e3).toFixed(1)} k`;
  return fmtInt(n);
}

function fmtClock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getUTCHours()).padStart(2, '0')}:${String(d.getUTCMinutes()).padStart(2, '0')}`;
}

function dayLabel(date: string): string {
  const [y, m, d] = date.split('-').map(Number);
  return `${d} ${new Date(Date.UTC(y!, m! - 1, d)).toLocaleString('en', { month: 'short', timeZone: 'UTC' })}`;
}

function dayStartMs(date: string): number {
  const [y, m, d] = date.split('-').map(Number);
  return Date.UTC(y!, m! - 1, d);
}

function isWeekend(date: string): boolean {
  const day = new Date(dayStartMs(date)).getUTCDay();
  return day === 0 || day === 6;
}

function stamp(ms: number, span: Span): string {
  const day = new Date(ms).toISOString().slice(0, 10);
  return span.from === span.to ? fmtClock(ms) : `${dayLabel(day)} ${fmtClock(ms)}`;
}

const str = (value: unknown) => (typeof value === 'string' ? value : value == null ? '' : String(value));
const num = (value: unknown) => (typeof value === 'number' ? value : 0);

// ---- kinds --------------------------------------------------------------------------------------

/** Every kind of failure an app records, in the order they are drawn, and its colour. */
const ERROR_KINDS: Array<[string, string]> = [
  ['panic', 'panic'],
  ['error', 'error'],
  ['warning', 'warning'],
  ['http_status', 'failed answer'],
  ['log_flood', 'log flood'],
  ['ui_render', 'view failed'],
  ['ui_error', 'page error'],
  ['ui_rejection', 'unhandled rejection'],
  ['ui_console', 'console error'],
  ['ui_request', 'no answer'],
  ['collector_dropped', 'not collected'],
];

/** The kind field a table's rows are coloured and stacked by. */
const kindField = (table: Table) => (table === 'errors' ? 'kind' : 'outcome');

function kindLabel(table: Table, kind: string): string {
  if (table !== 'errors') return kind === 'ok' ? 'answered' : kind;
  return ERROR_KINDS.find(([key]) => key === kind)?.[1] ?? kind;
}

function kindOrder(table: Table, kind: string): number {
  if (table !== 'errors') return kind === 'ok' ? 0 : 1;
  const index = ERROR_KINDS.findIndex(([key]) => key === kind);
  return index === -1 ? ERROR_KINDS.length : index;
}

/** The class that gives a kind its colour (`flow.css`). */
const kindClass = (table: Table, kind: string) => (table !== 'errors' ? (kind === 'ok' ? 'cf-k-ok' : 'cf-k-failed') : `cf-k-${kind}`);

/** What a table's rows count. */
const unitOf = (table: Table) => (table === 'errors' ? 'failures' : table === 'models' ? 'model calls' : 'calls');

// ---- the data -----------------------------------------------------------------------------------

/** What the bar and the page share: the computers that sent, the chosen days, the answer. */
export function useFlow(table: Table | null, repo: string, node: string) {
  const [span, setSpan] = useState<Span | null>(null);
  useEffect(() => setSpan(null), [table]);
  const key = table ? `manage.reports.read|${table}|${repo}|${node}|${span?.from ?? ''}|${span?.to ?? ''}` : null;
  const answer = useQuery<Answer>(
    key,
    async (client: Client) => client.call<Answer>('manage.reports.read', { table, repo, node, from: span?.from ?? '', to: span?.to ?? '' }),
    { topics: [CHANGED] },
  );
  const [computers, setComputers] = useState<Computer[]>([]);
  useEffect(() => {
    if (answer.data) setComputers(answer.data.computers);
  }, [answer.data]);
  return { answer, span, setSpan, computers };
}

type Flow = ReturnType<typeof useFlow>;

type Filters = Record<string, string | null>;

const ERROR_FACETS: Array<[string, string]> = [
  ['kind', 'kind'],
  ['app', 'app'],
  ['source', 'source'],
  ['area', 'area'],
  ['version', 'version'],
  ['computer', 'computer'],
];

const MODEL_FACETS: Array<[string, string]> = [
  ['model_id', 'model'],
  ['tag', 'tag'],
  ['served_by', 'served by'],
  ['app', 'app'],
  ['part', 'part'],
  ['computer', 'asked from'],
];

/** A field's name where the page shows it. */
const FIELD_LABELS: Record<string, string> = Object.fromEntries([...MODEL_FACETS, ['outcome', 'outcome']]);

const CALL_FACETS: Array<[string, string]> = [
  ['outcome', 'outcome'],
  ['app', 'app'],
  ['part', 'part'],
  ['version', 'version'],
  ['computer', 'computer'],
];

function filterRows(rows: Row[], filters: Filters): Row[] {
  return rows.filter((row) => Object.entries(filters).every(([field, value]) => value == null || str(row[field]) === value));
}

function facetValues(rows: Row[], field: string, table: Table, weigh: (row: Row) => number = (row) => row.count): Array<{ value: string; count: number }> {
  const sums = new Map<string, number>();
  for (const row of rows) {
    const value = str(row[field]);
    if (!value) continue;
    sums.set(value, (sums.get(value) ?? 0) + weigh(row));
  }
  return [...sums.entries()]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => b.count - a.count || (field === kindField(table) ? kindOrder(table, a.value) - kindOrder(table, b.value) : a.value.localeCompare(b.value)));
}

// ---- the page -----------------------------------------------------------------------------------

export function FlowPage({ table, flow, openRequest, onView }: { table: Table; flow: Flow; openRequest: (request: RequestLink) => void; onView?: ((table: 'models' | 'calls') => void) | undefined }) {
  const { answer, setSpan } = flow;
  const [filters, setFilters] = useState<Filters>({});
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => {
    setFilters({});
    setOpen(null);
  }, [table]);
  const data = answer.data;
  const span: Span | null = flow.span ?? (data ? { from: data.today, to: data.today } : null);

  /* ← → move the chosen day */
  useEffect(() => {
    if (!data || !span) return undefined;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
      const target = event.target as Element | null;
      if (target?.closest?.('input, textarea, [contenteditable]')) return;
      const dates = data.days.map((day) => day.date);
      const next = dates[dates.indexOf(span.to) + (event.key === 'ArrowRight' ? 1 : -1)];
      if (!next) return;
      event.preventDefault();
      setOpen(null);
      setSpan({ from: next, to: next });
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [data, span, setSpan]);

  const pick = (date: string, extend: boolean) => {
    setOpen(null);
    if (!extend || !span) setSpan({ from: date, to: date });
    else setSpan(date < span.from ? { from: date, to: span.to } : { from: span.from, to: date });
  };
  const toggle = (field: string, value: string) => {
    setOpen(null);
    setFilters((current) => ({ ...current, [field]: current[field] === value ? null : value }));
  };

  const views = onView && (
    <div className="cm-flow-views">
      <button type="button" className="detent" aria-pressed={table === 'models'} onClick={() => onView('models')}>Model calls</button>
      <button type="button" className="detent" aria-pressed={table === 'calls'} onClick={() => onView('calls')}>Methods</button>
    </div>
  );
  if (!data || data.table !== table) {
    if (answer.error) return <div className="cm-flow">{views}<p className="inline-error cm-board-note">{loadFailure('The reports', answer)}</p></div>;
    return <div className="cm-flow">{views}<p className="cm-line">Reading the reports…</p></div>;
  }
  // The answer is for the days asked; while another span loads, the last one stays.
  const rows = filterRows(data.rows, filters);
  const chips = Object.entries(filters).filter(([, value]) => value != null) as Array<[string, string]>;
  return (
    <div className="cm-flow" data-table={table}>
      {views}
      <Days table={table} days={data.days} span={span!} onPick={pick} />
      {chips.length > 0 && (
        <div className="cm-flow-chain">
          {chips.map(([field, value]) => (
            <button key={field} type="button" className="detent" aria-pressed="true" onClick={() => toggle(field, value)}>
              {(table === 'models' ? FIELD_LABELS[field] : undefined) ?? field} · {field === kindField(table) ? kindLabel(table, value) : value} <span className="k">✕</span>
            </button>
          ))}
        </div>
      )}
      {table === 'errors'
        ? <ErrorBody rows={rows} all={data.rows} span={span!} filters={filters} toggle={toggle} open={open} setOpen={setOpen}
            requests={data.requests ?? {}} openRequest={openRequest} refresh={() => void answer.refresh()} />
        : table === 'models'
          ? <ModelBody rows={rows} all={data.rows} span={span!} filters={filters} toggle={toggle} open={open} setOpen={setOpen} />
          : <CallBody rows={rows} all={data.rows} span={span!} filters={filters} toggle={toggle} open={open} setOpen={setOpen} />}
    </div>
  );
}

interface BodyProps {
  requests?: Record<string, RequestLink>;
  openRequest?: (request: RequestLink) => void;
  refresh?: () => void;
  rows: Row[];
  all: Row[];
  span: Span;
  filters: Filters;
  toggle: (field: string, value: string) => void;
  open: string | null;
  setOpen: (key: string | null) => void;
}

function Days({ table, days, span, onPick }: { table: Table; days: Day[]; span: Span; onPick: (date: string, extend: boolean) => void }) {
  const max = Math.max(1, ...days.map((day) => day.count));
  const what = unitOf(table);
  return (
    <div className="cm-flow-days">
      <div className="cf-strip">
        {days.map((day) => {
          const selected = day.date >= span.from && day.date <= span.to;
          const kinds = Object.entries(day.by_kind).sort((a, b) => kindOrder(table, a[0]) - kindOrder(table, b[0]));
          const title = day.groups === 0
            ? dayLabel(day.date)
            : `${dayLabel(day.date)} · ${fmtInt(day.count)} ${what} in ${fmtInt(day.groups)} rows\n${kinds.map(([kind, n]) => `${kindLabel(table, kind)} ${fmtInt(n)}`).join(' · ')}`;
          return (
            <button key={day.date} type="button" className={`cf-day${selected ? ' cf-sel' : ''}${day.groups === 0 ? ' cf-empty' : ''}${isWeekend(day.date) ? ' cf-wk' : ''}`} title={title}
              onClick={(event) => onPick(day.date, event.shiftKey)}>
              <span className="cf-stack" style={{ height: `${day.count ? Math.max(2, (64 * day.count) / max) : 0}px` }}>
                {kinds.map(([kind, n]) => <i key={kind} className={kindClass(table, kind)} style={{ flexGrow: n }} />)}
              </span>
              <span className={`pip${day.groups === 0 ? ' hollow' : ''}`} />
              <span className="cf-lbl num">{Number(day.date.slice(8))}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function Fig({ label, value, note, keyed, bad }: { label: string; value: string; note?: string; keyed?: boolean; bad?: boolean }) {
  return (
    <div className={`cf-fig${keyed ? ' cf-key' : ''}`}>
      <div className="ap faint">{label}</div>
      <div className="cf-v num">{value}</div>
      {note ? <div className={`cf-d${bad ? ' cf-bad' : ''}`}>{note}</div> : null}
    </div>
  );
}

/** A section, headed as the repo's pages head theirs: a pip, its name, what it holds. */
function Section({ title, count, tools, children }: { title: string; count?: string; tools?: ReactNode; children: ReactNode }) {
  return (
    <section className="cm-flow-sec">
      <div className="cm-flow-hd">
        <span className="pip hollow" />
        <span className="ap">{title}</span>
        {count && <span className="ap faint">· {count}</span>}
        <span className="grow" />
        {tools}
      </div>
      {children}
    </section>
  );
}

/** Stacked columns per slice of the days, one colour per kind. */
function Timeline({ table, rows, span, compact, onKind }: { table: Table; rows: Row[]; span: Span; compact?: boolean; onKind?: (kind: string) => void }) {
  const from = dayStartMs(span.from);
  const to = dayStartMs(span.to) + DAY_MS;
  const days = Math.round((to - from) / DAY_MS);
  const count = compact ? 96 : days === 1 ? 288 : Math.min(672, days * 96);
  const field = kindField(table);
  const slices = useMemo(() => {
    const width = (to - from) / count;
    const out = Array.from({ length: count }, (_, index) => ({ t0: from + index * width, t1: from + (index + 1) * width, total: 0, byKind: new Map<string, number>() }));
    for (const row of rows) {
      const index = Math.floor((row.first_at_ms - from) / width);
      const slice = out[index];
      if (!slice) continue;
      slice.total += row.count;
      const kind = str(row[field]);
      slice.byKind.set(kind, (slice.byKind.get(kind) ?? 0) + row.count);
    }
    return out;
  }, [rows, from, to, count, field]);
  const max = Math.max(1, ...slices.map((slice) => slice.total));
  const H = compact ? 60 : 200;
  const w = 1000 / count;
  const ticks: number[] = [];
  const step = days === 1 ? 3 * 3_600_000 : days <= 3 ? 12 * 3_600_000 : DAY_MS;
  for (let t = from; t <= to; t += step) ticks.push(t);
  const what = unitOf(table);
  return (
    <div className={`cm-flow-plot${compact ? ' cf-compact' : ''}`}>
      <span className="cf-max num">{fmtInt(max)}</span>
      <svg viewBox={`0 0 1000 ${H}`} preserveAspectRatio="none" role="img" aria-label={`${what} by time`}>
        <line className="cf-grid" x1="0" x2="1000" y1="0.5" y2="0.5" />
        <line className="cf-grid cf-major" x1="0" x2="1000" y1={H - 0.5} y2={H - 0.5} />
        {slices.map((slice, index) => {
          if (!slice.total) return null;
          let base = H;
          const kinds = [...slice.byKind.entries()].sort((a, b) => kindOrder(table, a[0]) - kindOrder(table, b[0]));
          return (
            <g key={index}>
              <title>{`${fmtClock(slice.t0)}–${fmtClock(slice.t1)} UTC · ${fmtInt(slice.total)} ${what}\n${kinds.map(([kind, n]) => `${kindLabel(table, kind)} ${fmtInt(n)}`).join(' · ')}`}</title>
              {kinds.map(([kind, n]) => {
                const h = Math.max(1, (n / max) * (H - 4));
                base -= h;
                return <rect key={kind} className={`${kindClass(table, kind)}${onKind ? ' cf-hit' : ''}`} x={index * w} y={base} width={Math.max(0.6, w * 0.8)} height={h} onClick={onKind ? () => onKind(kind) : undefined} />;
              })}
            </g>
          );
        })}
      </svg>
      {!compact && (
        <div className="cf-axis">
          {ticks.map((t) => (
            <span key={t} className="num" style={{ left: `${((t - from) / (to - from)) * 100}%` }}>
              {days === 1 ? fmtClock(t) : days <= 3 ? `${new Date(t).getUTCDate()} ${fmtClock(t)}` : String(new Date(t).getUTCDate())}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** The kinds there are; each one filters the page to it, pressed again shows them all. */
function Legend({ table, all, filters, toggle }: { table: Table; all: Row[]; filters: Filters; toggle: (field: string, value: string) => void }) {
  const field = kindField(table);
  const kinds = [...new Set(all.map((row) => str(row[field])))].sort((a, b) => kindOrder(table, a) - kindOrder(table, b));
  if (kinds.length === 0) return null;
  const chosen = filters[field] ?? null;
  return (
    <div className="cm-flow-legend">
      {kinds.map((kind) => (
        <button key={kind} type="button" className={`cf-key-btn${chosen === kind ? ' cf-on' : ''}${chosen && chosen !== kind ? ' cf-off' : ''}`} aria-pressed={chosen === kind} onClick={() => toggle(field, kind)}>
          <i className={`cf-sw ${kindClass(table, kind)}`} />{kindLabel(table, kind)}
        </button>
      ))}
    </div>
  );
}

function Facets({ table, facets, all, filters, toggle, unit, weigh, format = fmtInt }: { table: Table; facets: Array<[string, string]>; all: Row[]; filters: Filters; toggle: (field: string, value: string) => void; unit: string; weigh?: (row: Row) => number; format?: (n: number) => string }) {
  return (
    <div className="cm-flow-facets" style={{ gridTemplateColumns: `repeat(${facets.length}, minmax(0, 1fr))` }}>
      {facets.map(([field, label]) => (
        <div key={field} className="cf-rank">
          <h4><span className="ap">{label}</span><span className="ap faint">{unit}</span></h4>
          {facetValues(all, field, table, weigh).map((entry) => (
            <button key={entry.value} type="button" className={`cf-row${filters[field] === entry.value ? ' cf-on' : ''}`} title={entry.value} onClick={() => toggle(field, entry.value)}>
              <span className="cf-n">
                {field === kindField(table) && <i className={`cf-sw ${kindClass(table, entry.value)}`} />}
                {field === kindField(table) ? kindLabel(table, entry.value) : entry.value}
              </span>
              <span className="cf-c num">{format(entry.count)}</span>
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

// ---- Error flow ---------------------------------------------------------------------------------

interface Place {
  key: string;
  kind: string;
  source: string;
  area: string;
  target: string;
  location: string;
  level: string;
  message: string;
  count: number;
  first: number;
  last: number;
  apps: string[];
  versions: string[];
  computers: string[];
  samples: string[];
  rows: Row[];
}

/** One place: every row of a fingerprint in one app over the chosen days, summed. */
function places(rows: Row[]): Place[] {
  const byPlace = new Map<string, Place & { appSet: Set<string>; versionSet: Set<string>; computerSet: Set<string> }>();
  for (const row of [...rows].sort((a, b) => a.first_at_ms - b.first_at_ms)) {
    const key = `${str(row.repo)}|${str(row.fingerprint)}`;
    let place = byPlace.get(key);
    if (!place) {
      place = {
        key,
        kind: str(row.kind),
        source: str(row.source),
        area: str(row.area),
        target: str(row.target),
        location: str(row.location),
        level: str(row.level),
        message: str(row.message),
        count: 0,
        first: row.first_at_ms,
        last: row.last_at_ms,
        apps: [],
        versions: [],
        computers: [],
        samples: [],
        rows: [],
        appSet: new Set(),
        versionSet: new Set(),
        computerSet: new Set(),
      };
      byPlace.set(key, place);
    }
    place.count += row.count;
    place.first = Math.min(place.first, row.first_at_ms);
    place.last = Math.max(place.last, row.last_at_ms);
    if (row.app) place.appSet.add(row.app);
    if (row.version) place.versionSet.add(row.version);
    if (row.computer) place.computerSet.add(row.computer);
    const samples = Array.isArray(row.samples) && row.samples.length ? (row.samples as unknown[]).map(str) : [str(row.message)];
    for (const sample of samples) if (sample && !place.samples.includes(sample)) place.samples.push(sample);
    place.rows.push(row);
  }
  return [...byPlace.values()]
    .map(({ appSet, versionSet, computerSet, ...place }) => ({ ...place, apps: [...appSet].sort(), versions: [...versionSet].sort(), computers: [...computerSet].sort() }))
    .sort((a, b) => b.count - a.count || b.last - a.last || a.location.localeCompare(b.location));
}

function ErrorBody({ rows, all, span, filters, toggle, open, setOpen, requests = {}, openRequest, refresh }: BodyProps) {
  const fig = useMemo(() => {
    const f = { failures: 0, places: new Set<string>(), server: 0, iface: 0, panics: 0, floods: 0, versions: new Set<string>() };
    for (const row of rows) {
      f.failures += row.count;
      f.places.add(`${str(row.repo)}|${str(row.fingerprint)}`);
      if (row.source === 'interface') f.iface += row.count;
      else f.server += row.count;
      if (row.kind === 'panic') f.panics += row.count;
      if (row.kind === 'log_flood') f.floods += 1;
      if (row.version) f.versions.add(row.version);
    }
    return f;
  }, [rows]);
  const list = useMemo(() => places(rows), [rows]);
  const days = Math.round((dayStartMs(span.to) - dayStartMs(span.from)) / DAY_MS) + 1;
  const versions = [...fig.versions].sort();
  const filtered = Object.values(filters).some((value) => value != null);
  return (
    <>
      <div className="cm-flow-figs">
        <Fig label="Failures" value={fmtInt(fig.failures)} note={`${fmtInt(rows.length)} rows`} keyed />
        <Fig label="Places" value={fmtInt(fig.places.size)} />
        <Fig label="Server" value={fmtInt(fig.server)} />
        <Fig label="Interface" value={fmtInt(fig.iface)} />
        <Fig label="Panics" value={fmtInt(fig.panics)} bad={fig.panics > 0} />
        <Fig label="Log floods" value={fmtInt(fig.floods)} />
        <Fig label="Versions" value={fmtInt(versions.length)} note={versions.join(', ')} />
        <Fig label="Days" value={fmtInt(days)} />
      </div>
      <Section title="When" count={fmtInt(rows.reduce((sum, row) => sum + row.count, 0))}>
        <Timeline table="errors" rows={rows} span={span} onKind={(kind) => toggle('kind', kind)} />
        <Legend table="errors" all={all} filters={filters} toggle={toggle} />
      </Section>
      <Section title="Filters">
        <Facets table="errors" facets={ERROR_FACETS} all={all} filters={filters} toggle={toggle} unit="failures" />
      </Section>
      <Section title="Places" count={fmtInt(list.length)}>
        {list.length === 0 ? <p className="cm-flow-empty">Nothing failed in these days{filtered ? ' under these filters' : ''}.</p> : (
          <div className="cm-flow-tbl-wrap">
            <table className="tbl cm-flow-tbl">
              <thead>
                <tr><th>kind</th><th>app</th><th>area</th><th>place</th><th className="cf-r">count</th><th>first</th><th>last</th><th>versions</th><th>message</th><th>request</th></tr>
              </thead>
              <tbody>
                {list.map((place) => (
                  <Fragment key={place.key}>
                    <tr className={`cf-rec${open === place.key ? ' cf-open' : ''}`} onClick={() => setOpen(open === place.key ? null : place.key)}>
                      <td className="k"><i className={`cf-sw cf-k-${place.kind}`} />{kindLabel('errors', place.kind)}</td>
                      <td className="cf-dim">{place.apps.join(', ')}</td>
                      <td className="cf-dim">{place.area}</td>
                      <td className="cf-mono cf-clip" title={place.location}>{place.location}</td>
                      <td className="cf-r num">{fmtInt(place.count)}</td>
                      <td className="num cf-dim">{stamp(place.first, span)}</td>
                      <td className="num cf-dim">{stamp(place.last, span)}</td>
                      <td className="cf-dim">{place.versions.join(', ')}</td>
                      <td className="cf-msg cf-clip" title={place.message}>{place.message}</td>
                      <td className={requests[place.key]?.open ? '' : 'cf-dim'}>{requests[place.key] ? `${requests[place.key]!.id} · ${requests[place.key]!.state}` : '—'}</td>
                    </tr>
                    {open === place.key && <tr><td colSpan={10} className="cf-detail-cell"><PlaceDetail place={place} span={span} request={requests[place.key]} openRequest={openRequest} refresh={refresh} /></td></tr>}
                  </Fragment>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </>
  );
}

/** What was going on: spans read as their names with fields, the rest as key / value. */
function scenarioLines(scenario: unknown): Array<[string, string]> {
  if (!scenario || typeof scenario !== 'object') return [];
  const out: Array<[string, string]> = [];
  for (const [key, value] of Object.entries(scenario as Record<string, unknown>)) {
    if (value == null || value === '') continue;
    if (key === 'spans' && Array.isArray(value)) {
      out.push([key, value.map((s: { name?: string; fields?: Record<string, unknown> }) => `${s.name ?? ''}${s.fields && Object.keys(s.fields).length ? ` ${JSON.stringify(s.fields)}` : ''}`).join(' › ')]);
    } else {
      out.push([key, typeof value === 'string' ? value : JSON.stringify(value)]);
    }
  }
  return out;
}

function PlaceDetail({ place, span, request, openRequest, refresh }: { place: Place; span: Span; request?: RequestLink | undefined; openRequest?: ((request: RequestLink) => void) | undefined; refresh?: (() => void) | undefined }) {
  const client = useClient();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [repo, fingerprint] = place.key.split('|') as [string, string];
  const make = async () => {
    setBusy(true);
    setFailed(null);
    try {
      const made = await client.call<{ id: string; repo: string; package?: string }>('manage.reports.request', { repo, fingerprint });
      refresh?.();
      openRequest?.({ id: made.id, repo: made.repo, package: made.package ?? '', open: true, state: '' });
    } catch (cause) {
      setFailed(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  const scenarios = useMemo(() => {
    const seen = new Set<string>();
    const out: Array<{ at: number; lines: Array<[string, string]>; fields: Record<string, unknown>; stack: string; computer: string }> = [];
    for (const row of place.rows) {
      const key = JSON.stringify(row.scenario ?? {});
      if (seen.has(key)) continue;
      seen.add(key);
      const fields = row.fields && typeof row.fields === 'object' ? (row.fields as Record<string, unknown>) : {};
      out.push({ at: row.first_at_ms, lines: scenarioLines(row.scenario), fields, stack: str(row.stack), computer: str(row.computer) });
    }
    return out;
  }, [place]);
  return (
    <div className="cm-flow-detail">
      <div className="cf-kv">
        <span className="cf-k">place</span><span>{place.location}</span>
        <span className="cf-k">target</span><span>{place.target}</span>
        <span className="cf-k">level</span><span>{place.level}</span>
        <span className="cf-k">source</span><span>{place.source}</span>
        <span className="cf-k">area</span><span>{place.area}</span>
        <span className="cf-k">app</span><span>{place.apps.join(', ')}</span>
        <span className="cf-k">versions</span><span>{place.versions.join(', ')}</span>
        <span className="cf-k">computers</span><span>{place.computers.join(', ')}</span>
        <span className="cf-k">seen</span><span>{fmtInt(place.count)} times, {stamp(place.first, span)} – {stamp(place.last, span)} UTC, in {fmtInt(place.rows.length)} minutes</span>
        <span className="cf-k">group</span><span>{fingerprint}</span>
        <span className="cf-k">request</span><span>{request ? `${request.id} · ${request.state}` : 'none'}</span>
      </div>
      <div className="cf-acts">
        {request?.open
          ? <button type="button" className="detent" onClick={() => openRequest?.(request)}>Open chat</button>
          : <button type="button" className="detent" disabled={busy} onClick={() => void make()}>{busy ? 'Opening' : 'Make a request'}</button>}
        {request && !request.open && <button type="button" className="detent" onClick={() => openRequest?.(request)}>Open {request.id}</button>}
      </div>
      {failed && <p className="inline-error">{failed}</p>}
      <Timeline table="errors" rows={place.rows} span={span} compact />
      <h5 className="ap faint">Messages</h5>
      {place.samples.map((sample, index) => <pre key={index}>{sample}</pre>)}
      <h5 className="ap faint">What was going on</h5>
      {scenarios.map((scenario, index) => (
        <div key={index} className="cf-scenario">
          <div className="cf-kv">
            <span className="cf-k">at</span><span>{stamp(scenario.at, span)} UTC{scenario.computer ? ` · ${scenario.computer}` : ''}</span>
            {scenario.lines.map(([key, value]) => <Fragment key={key}><span className="cf-k">{key}</span><span>{value}</span></Fragment>)}
            {Object.entries(scenario.fields).map(([key, value]) => <Fragment key={`f-${key}`}><span className="cf-k">{key}</span><span>{typeof value === 'string' ? value : JSON.stringify(value)}</span></Fragment>)}
          </div>
          {scenario.stack ? <pre className="cf-stack-text">{scenario.stack}</pre> : null}
        </div>
      ))}
    </div>
  );
}

// ---- Metrics flow -------------------------------------------------------------------------------

interface Method {
  key: string;
  method: string;
  app: string;
  part: string;
  calls: number;
  failed: number;
  totalUs: number;
  maxUs: number;
  prompt: number;
  cached: number;
  completion: number;
  last: number;
  outcomes: Map<string, number>;
  computers: Set<string>;
  versions: Set<string>;
  rows: Row[];
}

function methods(rows: Row[]): Method[] {
  const byMethod = new Map<string, Method>();
  for (const row of rows) {
    const key = `${str(row.app)}|${str(row.part)}|${str(row.method)}`;
    let entry = byMethod.get(key);
    if (!entry) {
      entry = { key, method: str(row.method), app: str(row.app), part: str(row.part), calls: 0, failed: 0, totalUs: 0, maxUs: 0, prompt: 0, cached: 0, completion: 0, last: 0, outcomes: new Map(), computers: new Set(), versions: new Set(), rows: [] };
      byMethod.set(key, entry);
    }
    entry.calls += row.count;
    if (row.outcome !== 'ok') entry.failed += row.count;
    entry.totalUs += num(row.total_us);
    entry.maxUs = Math.max(entry.maxUs, num(row.max_us));
    entry.prompt += num(row.prompt_tokens);
    entry.cached += num(row.cached_tokens);
    entry.completion += num(row.completion_tokens);
    entry.last = Math.max(entry.last, row.last_at_ms);
    const outcome = str(row.outcome);
    entry.outcomes.set(outcome, (entry.outcomes.get(outcome) ?? 0) + row.count);
    if (row.computer) entry.computers.add(row.computer);
    if (row.version) entry.versions.add(row.version);
    entry.rows.push(row);
  }
  return [...byMethod.values()];
}

type Sort = 'calls' | 'time' | 'longest' | 'failed' | 'tokens';

const SORTS: Array<[Sort, string]> = [
  ['calls', 'calls'],
  ['time', 'time spent'],
  ['longest', 'longest'],
  ['failed', 'failed'],
  ['tokens', 'tokens'],
];

function sortMethods(list: Method[], sort: Sort): Method[] {
  const by: Record<Sort, (m: Method) => number> = {
    calls: (m) => m.calls,
    time: (m) => m.totalUs,
    longest: (m) => m.maxUs,
    failed: (m) => m.failed,
    tokens: (m) => m.prompt + m.completion,
  };
  return [...list].sort((a, b) => by[sort](b) - by[sort](a) || b.calls - a.calls || a.method.localeCompare(b.method));
}

function CallBody({ rows, all, span, filters, toggle, open, setOpen }: BodyProps) {
  const [sort, setSort] = useState<Sort>('calls');
  const fig = useMemo(() => {
    const f = { calls: 0, failed: 0, totalUs: 0, maxUs: 0, prompt: 0, cached: 0, completion: 0, methods: new Set<string>(), versions: new Set<string>() };
    for (const row of rows) {
      f.calls += row.count;
      if (row.outcome !== 'ok') f.failed += row.count;
      f.totalUs += num(row.total_us);
      f.maxUs = Math.max(f.maxUs, num(row.max_us));
      f.prompt += num(row.prompt_tokens);
      f.cached += num(row.cached_tokens);
      f.completion += num(row.completion_tokens);
      f.methods.add(str(row.method));
      if (row.version) f.versions.add(row.version);
    }
    return f;
  }, [rows]);
  const list = useMemo(() => sortMethods(methods(rows), sort), [rows, sort]);
  const days = Math.round((dayStartMs(span.to) - dayStartMs(span.from)) / DAY_MS) + 1;
  const versions = [...fig.versions].sort();
  const filtered = Object.values(filters).some((value) => value != null);
  return (
    <>
      <div className="cm-flow-figs">
        <Fig label="Calls" value={fmtInt(fig.calls)} note={`${fmtInt(rows.length)} rows`} keyed />
        <Fig label="Methods" value={fmtInt(fig.methods.size)} />
        <Fig label="Failed" value={fmtInt(fig.failed)} bad={fig.failed > 0} note={fig.calls ? `${((100 * fig.failed) / fig.calls).toFixed(1)} % of calls` : undefined} />
        <Fig label="Mean time" value={fig.calls ? fmtUs(fig.totalUs / fig.calls) : '—'} note={`${fmtUs(fig.totalUs)} in all`} />
        <Fig label="Longest" value={fig.maxUs ? fmtUs(fig.maxUs) : '—'} />
        {fig.prompt + fig.completion > 0 && <Fig label="Tokens in" value={fmtTokens(fig.prompt)} note={fig.cached ? `${fmtTokens(fig.cached)} cached` : undefined} />}
        {fig.prompt + fig.completion > 0 && <Fig label="Tokens out" value={fmtTokens(fig.completion)} />}
        <Fig label="Versions" value={fmtInt(versions.length)} note={versions.join(', ')} />
        <Fig label="Days" value={fmtInt(days)} />
      </div>
      <Section title="When" count={fmtInt(rows.reduce((sum, row) => sum + row.count, 0))}>
        <Timeline table="calls" rows={rows} span={span} onKind={(kind) => toggle('outcome', kind)} />
        <Legend table="calls" all={all} filters={filters} toggle={toggle} />
      </Section>
      <Section title="Filters">
        <Facets table="calls" facets={CALL_FACETS} all={all} filters={filters} toggle={toggle} unit="calls" />
      </Section>
      <Section title="Methods" count={`${fmtInt(list.length)}`} tools={SORTS.map(([key, label]) => (
        <button key={key} type="button" className="detent" aria-pressed={sort === key} onClick={() => setSort(key)}>{label}</button>
      ))}>
        {list.length === 0 ? <p className="cm-flow-empty">No call was served in these days{filtered ? ' under these filters' : ''}.</p> : (
          <div className="cm-flow-tbl-wrap">
            <table className="tbl cm-flow-tbl">
              <thead>
                <tr><th>method</th><th>app</th><th>part</th><th className="cf-r">calls</th><th className="cf-r">failed</th><th className="cf-r">mean</th><th className="cf-r">longest</th><th className="cf-r">time spent</th><th className="cf-r">tokens in</th><th className="cf-r">tokens out</th><th>last</th></tr>
              </thead>
              <tbody>
                {list.map((entry) => (
                  <Fragment key={entry.key}>
                    <tr className={`cf-rec${open === entry.key ? ' cf-open' : ''}`} onClick={() => setOpen(open === entry.key ? null : entry.key)}>
                      <td className="cf-mono cf-clip" title={entry.method}><i className={`cf-sw ${entry.failed ? 'cf-k-failed' : 'cf-k-ok'}`} />{entry.method}</td>
                      <td className="cf-dim">{entry.app}</td>
                      <td className="cf-dim">{entry.part}</td>
                      <td className="cf-r num">{fmtInt(entry.calls)}</td>
                      <td className={`cf-r num${entry.failed ? ' cf-bad' : ' cf-dim'}`}>{entry.failed ? fmtInt(entry.failed) : '—'}</td>
                      <td className="cf-r num">{fmtUs(entry.totalUs / Math.max(1, entry.calls))}</td>
                      <td className="cf-r num cf-dim">{fmtUs(entry.maxUs)}</td>
                      <td className="cf-r num cf-dim">{fmtUs(entry.totalUs)}</td>
                      <td className="cf-r num cf-dim">{entry.prompt ? fmtTokens(entry.prompt) : '—'}</td>
                      <td className="cf-r num cf-dim">{entry.completion ? fmtTokens(entry.completion) : '—'}</td>
                      <td className="num cf-dim">{stamp(entry.last, span)}</td>
                    </tr>
                    {open === entry.key && <tr><td colSpan={11} className="cf-detail-cell"><MethodDetail entry={entry} span={span} /></td></tr>}
                  </Fragment>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </>
  );
}

function MethodDetail({ entry, span }: { entry: Method; span: Span }) {
  const outcomes = [...entry.outcomes.entries()].sort((a, b) => kindOrder('calls', a[0]) - kindOrder('calls', b[0]) || b[1] - a[1]);
  const byComputer = new Map<string, { calls: number; us: number }>();
  for (const row of entry.rows) {
    const key = `${str(row.computer)} · ${str(row.version)}`;
    const current = byComputer.get(key) ?? { calls: 0, us: 0 };
    current.calls += row.count;
    current.us += num(row.total_us);
    byComputer.set(key, current);
  }
  return (
    <div className="cm-flow-detail">
      <div className="cf-kv">
        <span className="cf-k">method</span><span>{entry.method}</span>
        <span className="cf-k">app</span><span>{entry.app} · {entry.part}</span>
        <span className="cf-k">outcomes</span><span>{outcomes.map(([outcome, n]) => `${kindLabel('calls', outcome)} ${fmtInt(n)}`).join(' · ')}</span>
        <span className="cf-k">served</span><span>{fmtInt(entry.calls)} times in {fmtInt(entry.rows.length)} minutes, the last at {stamp(entry.last, span)} UTC</span>
        {(entry.prompt > 0 || entry.completion > 0) && <><span className="cf-k">tokens</span><span>{fmtInt(entry.prompt)} in ({fmtInt(entry.cached)} cached) · {fmtInt(entry.completion)} out</span></>}
      </div>
      <Timeline table="calls" rows={entry.rows} span={span} compact />
      <h5 className="ap faint">Where</h5>
      <div className="cf-kv">
        {[...byComputer.entries()].sort((a, b) => b[1].calls - a[1].calls).map(([key, value]) => (
          <Fragment key={key}><span className="cf-k">{key}</span><span>{fmtInt(value.calls)} calls · mean {fmtUs(value.us / Math.max(1, value.calls))}</span></Fragment>
        ))}
      </div>
    </div>
  );
}

// ---- Model calls --------------------------------------------------------------------------------

/** Calls drawn in the table at most; the figures and filters count every one. */
const SHOWN_CALLS = 500;

const tokensOf = (row: Row) => num(row.prompt_tokens) + num(row.completion_tokens);

function fmtMs(ms: number): string {
  return fmtUs(ms * 1000);
}

/** The model as the row names it; a call no claim placed ran on the app's own model. */
const modelOf = (row: Row) => str(row.model_id) || str(row.plugin) || 'own model';

function ModelBody({ rows, all, span, filters, toggle, open, setOpen }: BodyProps) {
  const fig = useMemo(() => {
    const f = { calls: 0, failed: 0, prompt: 0, cached: 0, completion: 0, reasoning: 0, wallMs: 0, decodeMs: 0, models: new Set<string>(), servers: new Set<string>() };
    for (const row of rows) {
      f.calls += 1;
      if (row.outcome !== 'ok') f.failed += 1;
      f.prompt += num(row.prompt_tokens);
      f.cached += num(row.cached_tokens);
      f.completion += num(row.completion_tokens);
      f.reasoning += num(row.reasoning_tokens);
      f.wallMs += num(row.wall_ms);
      f.decodeMs += num(row.decode_ms);
      f.models.add(modelOf(row));
      if (row.served_by) f.servers.add(str(row.served_by));
    }
    return f;
  }, [rows]);
  const ordered = useMemo(() => [...rows].sort((a, b) => b.last_at_ms - a.last_at_ms), [rows]);
  const shown = ordered.slice(0, SHOWN_CALLS);
  const days = Math.round((dayStartMs(span.to) - dayStartMs(span.from)) / DAY_MS) + 1;
  const filtered = Object.values(filters).some((value) => value != null);
  return (
    <>
      <div className="cm-flow-figs">
        <Fig label="Model calls" value={fmtInt(fig.calls)} keyed />
        <Fig label="Tokens in" value={fmtTokens(fig.prompt)} note={`${fmtTokens(fig.cached)} cached`} />
        <Fig label="Tokens out" value={fmtTokens(fig.completion)} note={fig.decodeMs ? `${(fig.completion / (fig.decodeMs / 1000)).toFixed(0)} tok/s` : undefined} />
        <Fig label="Reasoning" value={fmtTokens(fig.reasoning)} note={fig.completion ? `${((100 * fig.reasoning) / fig.completion).toFixed(0)} % of out` : undefined} />
        <Fig label="Models" value={fmtInt(fig.models.size)} note={[...fig.models].sort().join(', ')} />
        <Fig label="Served by" value={fmtInt(fig.servers.size)} note={[...fig.servers].sort().join(', ')} />
        <Fig label="Failed" value={fmtInt(fig.failed)} bad={fig.failed > 0} />
        <Fig label="Mean time" value={fig.calls ? fmtMs(fig.wallMs / fig.calls) : '—'} note={`${fmtMs(fig.wallMs)} in all`} />
        <Fig label="Days" value={fmtInt(days)} />
      </div>
      <Section title="When" count={fmtInt(rows.length)}>
        <Timeline table="models" rows={rows} span={span} onKind={(kind) => toggle('outcome', kind)} />
        <Legend table="models" all={all} filters={filters} toggle={toggle} />
      </Section>
      <Section title="Who spends">
        <Facets table="models" facets={MODEL_FACETS} all={all} filters={filters} toggle={toggle} unit="tokens" weigh={tokensOf} format={fmtTokens} />
      </Section>
      <Section title="Calls" count={rows.length > SHOWN_CALLS ? `the last ${fmtInt(SHOWN_CALLS)} of ${fmtInt(rows.length)}` : fmtInt(rows.length)}>
        {rows.length === 0 ? <p className="cm-flow-empty">No model was called in these days{filtered ? ' under these filters' : ''}.</p> : (
          <div className="cm-flow-tbl-wrap">
            <table className="tbl cm-flow-tbl">
              <thead>
                <tr><th>ended</th><th>model</th><th>tag</th><th>served by</th><th>app</th><th>part</th><th className="cf-r">in</th><th className="cf-r">cached</th><th className="cf-r">out</th><th className="cf-r">reasoning</th><th className="cf-r">time</th><th>finish</th></tr>
              </thead>
              <tbody>
                {shown.map((row) => {
                  const key = str(row.event_id);
                  return (
                    <Fragment key={key}>
                      <tr className={`cf-rec${open === key ? ' cf-open' : ''}`} onClick={() => setOpen(open === key ? null : key)}>
                        <td className="num cf-dim">{stamp(row.last_at_ms, span)}</td>
                        <td className="cf-mono cf-clip" title={modelOf(row)}><i className={`cf-sw ${row.outcome === 'ok' ? 'cf-k-ok' : 'cf-k-failed'}`} />{modelOf(row)}</td>
                        <td className="cf-dim">{str(row.tag) || '—'}</td>
                        <td>{str(row.served_by)}{row.tier ? <span className="cf-dim"> · {str(row.tier)}</span> : null}</td>
                        <td className="cf-dim">{str(row.app)}</td>
                        <td className="cf-dim">{str(row.part)}</td>
                        <td className="cf-r num">{fmtInt(num(row.prompt_tokens))}</td>
                        <td className="cf-r num cf-dim">{fmtInt(num(row.cached_tokens))}</td>
                        <td className="cf-r num">{fmtInt(num(row.completion_tokens))}</td>
                        <td className="cf-r num cf-dim">{fmtInt(num(row.reasoning_tokens))}</td>
                        <td className="cf-r num cf-dim">{fmtMs(num(row.wall_ms))}</td>
                        <td className={row.outcome === 'ok' ? 'cf-dim' : 'cf-bad'}>{row.outcome === 'ok' ? str(row.finish) : str(row.outcome)}</td>
                      </tr>
                      {open === key && <tr><td colSpan={12} className="cf-detail-cell"><ModelDetail row={row} span={span} /></td></tr>}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </>
  );
}

function ModelDetail({ row, span }: { row: Row; span: Span }) {
  const decode = num(row.decode_ms);
  const lines: Array<[string, string]> = [
    ['model', `${modelOf(row)}${row.plugin && row.model_id ? ` · ${str(row.plugin)}` : ''}`],
    ['tag', str(row.tag) || 'not claimed'],
    ['served by', `${str(row.served_by)}${row.tier ? ` · ${str(row.tier)}` : ''}${row.served_node ? ` · ${str(row.served_node)}` : ''}`],
    ['asked by', `${str(row.app)} ${str(row.app_version)} · ${str(row.part)} · ${str(row.computer)}`],
    ['method', str(row.method)],
    ['tokens', `${fmtInt(num(row.prompt_tokens))} in (${fmtInt(num(row.cached_tokens))} cached) · ${fmtInt(num(row.completion_tokens))} out (${fmtInt(num(row.reasoning_tokens))} reasoning)`],
    ['times', `queue ${fmtMs(num(row.queue_ms))} · prefill ${fmtMs(num(row.prefill_ms))} · decode ${fmtMs(decode)}${decode ? ` (${(num(row.completion_tokens) / (decode / 1000)).toFixed(0)} tok/s)` : ''} · in all ${fmtMs(num(row.wall_ms))}`],
    ['ended', `${stamp(row.last_at_ms, span)} UTC · ${row.outcome === 'ok' ? str(row.finish) : str(row.outcome)}`],
  ];
  return (
    <div className="cm-flow-detail">
      <div className="cf-kv">
        {lines.map(([key, value]) => <Fragment key={key}><span className="cf-k">{key}</span><span>{value}</span></Fragment>)}
      </div>
    </div>
  );
}
