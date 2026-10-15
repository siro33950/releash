const { readFileSync, existsSync } = require('node:fs');
const { createHash } = require('node:crypto');

module.exports = async ({ github, context, core, evidence, artifactUrl, sha }) => {
  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  function readResult(path, fallback) {
    if (!existsSync(path)) return fallback;
    try { return JSON.parse(readFileSync(path, 'utf8')); }
    catch (error) { core.warning(`Cannot read ${path}: ${error.message}`); return fallback; }
  }
  const summary = readResult(`${evidence}/summary.json`, null);
  const failures = summary?.failedTests > 0 ? (summary.testFailures ?? []) : [];
  const tests = new Map(failures.map(failure => [
    `${failure.targetName}/${failure.testIdentifierString}`, failure.failureText,
  ]));
  if (!tests.size) tests.set('runner-or-build', 'テスト結果を確定できませんでした。実行ログを確認してください。');
  const manifestPath = `${evidence}/screenshots/manifest.json`;
  const screenshots = readResult(manifestPath, []);
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns, {
    ...context.repo, workflow_id: 'nightly.yml', branch: 'main', status: 'success', per_page: 100,
  });
  const previous = runs.filter(run => run.id < context.runId)
    .sort((a, b) => b.id - a.id)[0];
  let commits = '成功した Nightly はまだありません。';
  if (previous) {
    const comparison = `${previous.head_sha}...${sha}`;
    const changes = await github.paginate(github.rest.repos.compareCommitsWithBasehead, {
      ...context.repo, basehead: comparison, per_page: 100,
    }, response => response.data.commits);
    commits = `[前回成功からの差分](${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/compare/${comparison})\n\n`
      + changes.map(commit => `- ${commit.sha}: ${commit.commit.message.split('\n')[0].slice(0, 500)}`).join('\n');
  }
  const commitChunks = [''];
  for (const line of commits.split('\n')) {
    if (commitChunks.at(-1).length + line.length > 40000) commitChunks.push('');
    commitChunks[commitChunks.length - 1] += `${line}\n`;
  }
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    ...context.repo, state: 'open', creator: 'github-actions[bot]', per_page: 100,
  });
  for (const [test, failure] of tests) {
    const marker = `<!-- macos-e2e:${createHash('sha256').update(test).digest('hex')} -->`;
    const attachments = screenshots.filter(entry => test.endsWith(entry.testIdentifier))
      .flatMap(entry => entry.attachments.map(attachment => `- \`screenshots/${attachment.exportedFileName}\``));
    const evidenceText = artifactUrl
      ? `[スクリーンショット・xcresult・ログを取得](${artifactUrl})\n${attachments.join('\n')}`
      : '証跡の保存に失敗しました。実行ログを確認してください。';
    const body = `${marker}\n回帰 E2E が失敗し、Nightly の公開を停止しました。\n\n`
      + `対象: \`${test}\`\ncommit: ${sha}\n[実行ログ](${runUrl})\n\n`
      + `失敗内容:\n\n\`\`\`text\n${failure.slice(0, 8000)}\n\`\`\`\n\n`
      + `${evidenceText}\n\n${commitChunks[0]}`;
    const existing = issues.find(issue => !issue.pull_request && issue.body?.includes(marker));
    let issueNumber = existing?.number;
    if (existing) {
      await github.rest.issues.createComment({ ...context.repo, issue_number: existing.number, body });
      core.info(`Updated E2E issue #${existing.number}`);
    } else {
      const { data } = await github.rest.issues.create({
        ...context.repo, title: `[Nightly E2E] ${test}`.slice(0, 256), body,
      });
      issueNumber = data.number;
      core.info(`Created E2E issue #${data.number}`);
    }
    for (const chunk of commitChunks.slice(1)) {
      await github.rest.issues.createComment({
        ...context.repo, issue_number: issueNumber,
        body: `[実行ログ](${runUrl})\n\n前回成功からの commit 一覧（続き）\n\n${chunk}`,
      });
    }
  }
};
