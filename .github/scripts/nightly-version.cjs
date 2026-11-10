const { retryCommit, requireSuccessfulCI } = require('./nightly-state.cjs');
const { execFileSync } = require('node:child_process');
const { readFileSync, appendFileSync } = require('node:fs');

function nextVersion(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Expected X.Y.Z');
  const [major, minor, patch] = version.split('.').map(Number);
  return `${major}.${minor}.${patch + 1}`;
}

function compareVersions(a, b) {
  return a.localeCompare(b, undefined, { numeric: true });
}

async function main(
  run = (...args) => execFileSync(args[0], args.slice(1), { encoding: 'utf8' }).trim(),
  mainVersion = () => readFileSync('Cargo.toml', 'utf8').match(/^version = "(.*)"/m)[1],
) {
  const api = path => JSON.parse(run('gh', 'api', path));
  const repo = process.env.GITHUB_REPOSITORY;
  const marker = `Nightly-Run: ${process.env.GITHUB_RUN_ID}`;
  const head = {
    sha: run('git', 'rev-parse', 'origin/main'),
    commit: { message: run('git', 'log', '-1', '--format=%B', 'origin/main') },
  };
  let sha = run('git', 'log', 'origin/main', '--format=%H', '--max-count=1', `--grep=^${marker}$`)
    || retryCommit([head], process.env.GITHUB_RUN_ID)?.sha;
  if (!sha) {
    const runs = api(`repos/${repo}/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=${head.sha}`).workflow_runs;
    requireSuccessfulCI(runs, head.sha);
    let latest;
    const drafts = [];
    for (let page = 1; ; page++) {
      const releases = JSON.parse(run('gh', 'api', '--jq', 'map({tag_name, draft})', `repos/${repo}/releases?per_page=30&page=${page}`));
      for (const release of releases) {
        if (release.draft) {
          drafts.push(release.tag_name);
          continue;
        }
        const version = release.tag_name.match(/^v(\d+\.\d+\.\d+)(?:-nightly\.\d{8}\.\d+)?$/)?.[1];
        if (version && (!latest || compareVersions(version, latest) > 0)) latest = version;
      }
      if (releases.length < 30) break;
    }
    const current = mainVersion();
    const version = latest && compareVersions(current, latest) > 0 ? current : nextVersion(latest || current);
    for (const tag of drafts.filter(tag => tag.match(/^v(\d+\.\d+\.\d+)-nightly\.\d{8}\.[1-9]\d*$/)?.[1] === version)) {
      run('gh', 'release', 'delete', tag, '--cleanup-tag', '--yes', '--repo', repo);
    }
    run('node', '.github/scripts/bump-version.cjs', version);
    run('git', 'config', 'user.name', 'github-actions[bot]');
    run('git', 'config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com');
    run('git', 'add', 'Cargo.toml', 'Cargo.lock', 'clients/macos/project.yml');
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
