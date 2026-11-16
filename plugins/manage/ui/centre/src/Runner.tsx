/**
 * A repository's Runner page: the computer that builds it, whether it answers, the jobs it holds,
 * and the repository's source and policy as they are set (changed in Settings > Repositories).
 */

import { useQuery } from '@cortex/sdk';
import { CHANGED, WATCH, count, loadFailure, minutesText, type Budget, type Repo } from './manage';

/** A job's facts as the runner reports them (the fields this page reads). */
interface Job {
  job_id: string;
  attempt: number;
  /** waiting | running | done | failed | stopped | refused | lost. */
  state: string;
  stopped_because: string;
  task: number;
  tasks: number;
  now: string;
  minutes: number;
  total_files: number;
  total_lines: number;
  error: string;
}

interface RunnerAnswer {
  reachable: boolean;
  node: string;
  name: string;
  jobs: Job[];
}

const JOB_STATE: Record<string, string> = {
  waiting: 'Waiting',
  running: 'Running',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
  refused: 'Refused',
  lost: 'Lost',
};

function jobLine(job: Job): string {
  const parts = [job.job_id];
  if (job.tasks > 0) parts.push(`task ${job.task} of ${job.tasks}`);
  if (job.attempt > 1) parts.push(`attempt ${job.attempt}`);
  if (job.minutes > 0) parts.push(`${job.minutes} min`);
  if (job.total_files > 0) parts.push(`${count(job.total_files, 'file')}, ${count(job.total_lines, 'line')}`);
  return parts.join(' · ');
}

function budgetText(budget: Budget): string {
  return `${budget[0]} min · ${budget[1].toLocaleString()} tokens · ${budget[2].toLocaleString()} lines`;
}

export function RunnerPage({ repoId, repo }: { repoId: string; repo: Repo | undefined }) {
  const runner = useQuery<RunnerAnswer>(
    `manage.runner.get:${repoId}`,
    (client) => client.call<RunnerAnswer>('manage.runner.get', { repo: repoId }),
    { topics: [CHANGED, WATCH] },
  );
  const data = runner.data;
  const jobs = data?.jobs ?? [];
  const policy = repo?.policy;
  return (
    <div className="cm-page pad" data-page="runner">
      <div className="cm-sec">
        <span className="ap">Runner</span>
        {runner.error && !data && <p className="inline-error">{loadFailure('The runner', runner)}</p>}
        {data && (
          <>
            <div className="cm-row"><span>Builds on</span><span>{data.name || repo?.runner_name || 'this computer'}</span></div>
            <div className="cm-row">
              <span>Answers</span>
              <span className={`ap ${data.reachable ? 'origin' : 'live'}`}>{data.reachable ? 'Reachable' : 'Not reachable'}</span>
            </div>
            {!data.reachable && <p className="cm-note faint">Nothing builds for this repository until its runner answers again.</p>}
          </>
        )}
      </div>
      <div className="cm-sec">
        <span className="ap">Jobs</span>
        {data && jobs.length === 0 && <p className="cm-note faint">No job is on the runner.</p>}
        {jobs.map((job) => (
          <div key={`${job.job_id}:${job.attempt}`}>
            <div className="cm-row">
              <span>{jobLine(job)}</span>
              <span className={`ap ${job.state === 'running' ? 'origin' : job.state === 'failed' || job.state === 'lost' || job.state === 'refused' ? 'live' : 'faint'}`}>{JOB_STATE[job.state] ?? job.state}</span>
            </div>
            {job.state === 'running' && job.now && <p className="cm-note faint">Now: {job.now}</p>}
            {job.error && <p className="cm-note faint">{job.error}</p>}
          </div>
        ))}
      </div>
      {repo && (
        <div className="cm-sec">
          <span className="ap">Source</span>
          <div className="cm-row"><span>Git</span><span className="mono">{repo.source.url}</span></div>
          <div className="cm-row"><span>Main branch</span><span className="mono">{repo.source.main}</span></div>
          <div className="cm-row"><span>A landed package</span><span>{repo.source.changes === 'github-pr' ? 'also opens a pull request' : 'goes straight to the main branch'}</span></div>
        </div>
      )}
      {policy && (
        <div className="cm-sec">
          <span className="ap">Policy</span>
          <div className="cm-row"><span>Hold</span><span>safe {minutesText(policy.hold_safe_min)} · minor {minutesText(policy.hold_minor_min)} · at most {minutesText(policy.hold_max_min)}</span></div>
          <div className="cm-row"><span>Soak</span><span>safe {minutesText(policy.soak_safe_min)} · minor {minutesText(policy.soak_minor_min)} · major {minutesText(policy.soak_major_min)}</span></div>
          <div className="cm-row"><span>Rework rounds</span><span>safe {policy.rework_safe} · minor {policy.rework_minor} · major {policy.rework_major}</span></div>
          <div className="cm-row"><span>Budget, safe</span><span>{budgetText(policy.budget_safe)}</span></div>
          <div className="cm-row"><span>Budget, minor</span><span>{budgetText(policy.budget_minor)}</span></div>
          <div className="cm-row"><span>Budget, major</span><span>{budgetText(policy.budget_major)}</span></div>
          <div className="cm-row"><span>Daily train</span><span className="num">{policy.train_at}</span></div>
        </div>
      )}
      {policy && (
        <div className="cm-sec">
          <span className="ap">Paths that make a change major</span>
          {policy.major_paths.length === 0 ? <p className="cm-note faint">None.</p> : <ul className="cm-paths">{policy.major_paths.map((glob) => <li key={glob}>{glob}</li>)}</ul>}
        </div>
      )}
      {policy && (
        <div className="cm-sec">
          <span className="ap">Paths that make a change at least minor</span>
          {policy.minor_paths.length === 0 ? <p className="cm-note faint">None.</p> : <ul className="cm-paths">{policy.minor_paths.map((glob) => <li key={glob}>{glob}</li>)}</ul>}
        </div>
      )}
    </div>
  );
}
