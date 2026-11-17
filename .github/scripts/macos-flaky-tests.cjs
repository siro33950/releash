const { execFileSync } = require('node:child_process');
const { existsSync, readdirSync } = require('node:fs');
const { createHash } = require('node:crypto');

function flakyTests(results) {
  const found = [];
  const visit = (node, bundle) => {
    if (node.nodeType === 'Test Case') {
      const failed = (node.children ?? [])
        .filter(child => child.nodeType === 'Repetition' && child.result === 'Failed');
      if (node.result === 'Passed' && failed.length) {
        found.push({
          test: `${bundle}/${node.nodeIdentifier ?? node.name}`,
          failure: failed.flatMap(run => (run.children ?? [])
            .filter(child => child.nodeType === 'Failure Message').map(child => child.name)).join('\n'),
        });
      }
      return;
    }
    const next = node.nodeType === 'Unit test bundle' || node.nodeType === 'UI test bundle' ? node.name : bundle;
    for (const child of node.children ?? []) visit(child, next);
  };
  for (const node of results.testNodes ?? []) visit(node, '');
  return found;
}

function detect({ core, directory, run = (...args) => execFileSync(args[0], args.slice(1), { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }) }) {
  const bundles = existsSync(directory) ? readdirSync(directory).filter(name => name.endsWith('.xcresult')).sort() : [];
  let tests = [];
  if (bundles.length) {
    try {
      const path = `${directory}/${bundles.at(-1)}`;
      tests = flakyTests(JSON.parse(run('xcrun', 'xcresulttool', 'get', 'test-results', 'tests', '--path', path, '--compact')));
    } catch (error) {
      core.warning(`Cannot read test results: ${error.message}`);
    }
  }
  core.setOutput('found', String(tests.length > 0));
  core.setOutput('tests', JSON.stringify(tests));
  return tests;
}

async function report({ github, context, core, tests, artifactUrl }) {
  if (context.eventName !== 'push') {
    for (const { test, failure } of tests) core.warning(`再試行で通った不安定なテスト: ${test}\n${failure}`);
    return;
  }
  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    ...context.repo, state: 'open', creator: 'github-actions[bot]', per_page: 100,
  });
  for (const { test, failure } of tests) {
    const marker = `<!-- macos-flaky:${createHash('sha256').update(test).digest('hex')} -->`;
    const evidence = artifactUrl ? `[スクリーンショット・xcresult を取得](${artifactUrl})` : '証跡の保存に失敗しました。実行ログを確認してください。';
    const body = `${marker}\nCI の macOS 統合テストで 1 回目に失敗し、再試行で通りました。\n\n`
      + `対象: \`${test}\`\ncommit: ${context.sha}\n[実行ログ](${runUrl})\n\n`
      + `失敗内容:\n\n\`\`\`text\n${failure.slice(0, 8000)}\n\`\`\`\n\n${evidence}\n`;
    const existing = issues.find(issue => !issue.pull_request && issue.body?.includes(marker));
    if (existing) {
      await github.rest.issues.createComment({ ...context.repo, issue_number: existing.number, body });
      core.info(`Updated flaky test issue #${existing.number}`);
    } else {
      const { data } = await github.rest.issues.create({
        ...context.repo, title: `[Flaky] ${test}`.slice(0, 256), body,
      });
      core.info(`Created flaky test issue #${data.number}`);
    }
  }
}

module.exports = { flakyTests, detect, report };
