const { retryCommit, requireSuccessfulCI } = require('./nightly-state.cjs');
const { execFileSync } = require('node:child_process');
const { readFileSync, appendFileSync } = require('node:fs');

function nextVersion(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Expected X.Y.Z');
  const [major, minor, patch] = version.split('.').map(Number);
  return `${major}.${minor}.${patch + 1}`;
}

async function main(run = (...args) => execFileSync(args[0], args.slice(1), { encoding: 'utf8' }).trim()) {
  const api = path => JSON.parse(run('gh', 'api', path));
  const repo = process.env.GITHUB_REPOSITORY;
  const marker = `Nightly-Run: ${process.env.GITHUB_RUN_ID}`;
  const commits = JSON.parse(run('gh', 'api', '--paginate', '--slurp', `repos/${repo}/commits?sha=main&per_page=100`)).flat();
  let sha = retryCommit(commits, process.env.GITHUB_RUN_ID)?.sha;
  if (!sha) {
    const runs = api(`repos/${repo}/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=${commits[0].sha}`).workflow_runs;
    requireSuccessfulCI(runs, commits[0].sha);
    const releases = JSON.parse(run('gh', 'api', '--paginate', '--slurp', `repos/${repo}/releases?per_page=100`)).flat();
    const published = releases.filter(release => !release.draft)
      .map(release => release.tag_name.match(/^v(\d+\.\d+\.\d+)(?:-nightly\.\d{8}\.\d+)?$/)?.[1]).filter(Boolean)
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    const version = nextVersion(published.at(-1) || JSON.parse(readFileSync('clients/desktop/package.json')).version);
    for (const release of releases.filter(release => release.draft && release.tag_name.match(/^v(\d+\.\d+\.\d+)-nightly\.\d{8}\.[1-9]\d*$/)?.[1] === version)) {
      run('gh', 'release', 'delete', release.tag_name, '--cleanup-tag', '--yes', '--repo', repo);
    }
    run('node', '.github/scripts/bump-version.cjs', version);
    run('git', 'config', 'user.name', 'github-actions[bot]');
    run('git', 'config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com');
    run('git', 'add', 'Cargo.toml', 'Cargo.lock', 'clients/desktop/package.json', 'clients/desktop/tauri.conf.json', 'clients/macos/project.yml');
    run('git', 'commit', '--allow-empty', '-m', `release: v${version}\n\n${marker}`);
    sha = run('git', 'rev-parse', 'HEAD');
    run('git', 'push', 'origin', 'HEAD:main');
  }
  appendFileSync(process.env.GITHUB_OUTPUT, `sha=${sha}\n`);
  const deadline = Date.now() + 45 * 60 * 1000;
  while (Date.now() < deadline) {
    const runs = api(`repos/${repo}/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=${sha}`).workflow_runs;
    if (runs.some(run => run.conclusion === 'success')) return;
    if (runs.length && runs.every(run => run.status === 'completed')) throw new Error(`CI failed for ${sha}`);
    await new Promise(resolve => setTimeout(resolve, 15000));
  }
  throw new Error(`CI did not succeed for ${sha}`);
}

module.exports = { nextVersion, main };
if (require.main === module) main().catch(error => { console.error(error); process.exitCode = 1; });
