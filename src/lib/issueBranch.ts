export function generateIssueBranchName(issueNumber: number): string {
	return `feat/issues/${issueNumber}`;
}
