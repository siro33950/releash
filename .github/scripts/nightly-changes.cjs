module.exports = async ({ github, context, core }) => {
  const { data: main } = await github.rest.repos.getCommit({
    ...context.repo, ref: 'heads/main',
  });
  core.setOutput('sha', main.sha);
  const commits = await github.paginate(github.rest.repos.listCommits, { ...context.repo, sha: 'main', per_page: 100 });
  const retry = commits.find(commit => commit.commit.message.split('\n').includes(`Nightly-Run: ${context.runId}`));
  if (retry) {
    core.setOutput('sha', retry.sha);
    core.setOutput('run', true);
    return;
  }
  if (context.eventName !== 'workflow_dispatch') {
    const releases = await github.paginate(github.rest.repos.listReleases, {
      ...context.repo, per_page: 100,
    });
    const latest = releases
      .filter(release => !release.draft && release.prerelease && /^v\d+\.\d+\.\d+-nightly\.\d{8}\.[1-9]\d*$/.test(release.tag_name))
      .sort((a, b) => b.published_at.localeCompare(a.published_at) || b.id - a.id)[0];
    const previous = latest && (await github.rest.repos.getCommit({
      ...context.repo, ref: latest.tag_name,
    })).data.sha;
    if (previous === main.sha) {
      core.setOutput('run', false);
      return;
    }
  }
  // PR 層の検証は main push の CI が済ませている。Nightly では繰り返さず、成功した run があることだけ確かめる
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns, {
    ...context.repo, workflow_id: 'ci.yml', branch: 'main', event: 'push', head_sha: main.sha, per_page: 100,
  });
  if (!runs.some(run => run.conclusion === 'success')) {
    const states = runs.map(run => `${run.id}: ${run.status}/${run.conclusion}`).join(', ') || 'none';
    throw new Error(`No successful CI run on main for ${main.sha} (${states}). Wait for CI or fix main before starting Nightly.`);
  }
  core.setOutput('run', true);
};
