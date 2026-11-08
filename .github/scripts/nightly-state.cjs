function retryCommit(commits, runId) {
  const retry = commits.find(commit => commit.commit.message.split('\n').includes(`Nightly-Run: ${runId}`));
  if (retry) return { sha: retry.sha, sameRun: true };
  const head = commits[0];
  if (head && /^release: v\d+\.\d+\.\d+\n/.test(head.commit.message)
      && /^Nightly-Run: \d+$/m.test(head.commit.message)) return { sha: head.sha, sameRun: false };
}

function requireSuccessfulCI(runs, sha) {
  if (!runs.some(run => run.conclusion === 'success')) {
    throw new Error(`No successful CI run on main: CI has not succeeded for ${sha}`);
  }
}

module.exports = { retryCommit, requireSuccessfulCI };
