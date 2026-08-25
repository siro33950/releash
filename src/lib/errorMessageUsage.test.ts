import { describe, expect, it } from "vitest";

const TYPESCRIPT_SOURCES = import.meta.glob(
	["./**/*.{ts,tsx}", "../**/*.{ts,tsx}"],
	{
		query: "?raw",
		import: "default",
		eager: true,
	},
);

const TARGET_FILES = [
	"src/components/workspace/WorkspaceList.tsx",
	"src/hooks/useProviderAvailabilitySettings.ts",
	"src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx",
	"src/components/panels/MarkdownDiffViewer.tsx",
	"src/hooks/useReviewFileView.ts",
	"src/lib/telemetry.ts",
	"src/hooks/useTerminal.ts",
	"src/components/panels/NodeContentView/NodeContentView.tsx",
	"src/hooks/useWorkspaceNodeDetail.ts",
	"src/components/panels/automation/FacetEditor.tsx",
	"src/components/panels/automation/NameInputDialog.tsx",
	"src/lib/workflowExecutionActions.ts",
	"src/hooks/useUpdateChecker.ts",
	"src/components/panels/SettingsModal.tsx",
	"src/hooks/useAutomation.ts",
	"src/screens/useWorktreeGitActions.ts",
	"src/hooks/useWorkflowConfig.ts",
	"src/hooks/useAppSettings.ts",
	"src/components/workspace/DeleteWorktreeDialog.tsx",
	"src/components/panels/DiffToolbar.tsx",
	"src/hooks/useWorkspaceTreeNodes.ts",
	"src/contexts/ReviewThreadHandoffContext.tsx",
	"src/hooks/useNotionSettings.ts",
	"src/hooks/useApplicationShutdownSupervision.ts",
] as const;

const LOCAL_STRING_EXTRACTION =
	/\bString\s*\(\s*(?:e|err|error|reason|cause|exception|caught)\s*\)/;
const LOCAL_ERROR_MESSAGE_TERNARY =
	/instanceof\s+Error\s*\?[\s\S]{0,120}?\.message\b/;
const LOCAL_HELPER_DECLARATION =
	/(?:function|const|let|var)\s+getErrorMessage\b/;
const GET_ERROR_MESSAGE_CALL = /\bgetErrorMessage\s*\(/;

function sourceFor(targetFile: string): string | undefined {
	const relativePath = targetFile.startsWith("src/lib/")
		? targetFile.replace(/^src\/lib\//, "./")
		: targetFile.replace(/^src\//, "../");
	return TYPESCRIPT_SOURCES[relativePath];
}

function targetFileFor(relativePath: string): string {
	return relativePath.startsWith("./")
		? `src/lib/${relativePath.slice(2)}`
		: `src/${relativePath.slice(3)}`;
}

describe("backend error message extraction", () => {
	it("対象ファイル一覧がproductionの共通抽出関数利用箇所と一致する", () => {
		const actualTargetFiles = Object.entries(TYPESCRIPT_SOURCES)
			.filter(
				([file, source]) =>
					!file.includes(".test.") &&
					!file.endsWith("/errorMessage.ts") &&
					GET_ERROR_MESSAGE_CALL.test(source),
			)
			.map(([file]) => targetFileFor(file))
			.sort();
		expect(actualTargetFiles).toEqual([...TARGET_FILES].sort());

		for (const file of TARGET_FILES) {
			const source = sourceFor(file);
			expect(
				source,
				`${file} must be part of the source inventory`,
			).toBeDefined();
			if (source === undefined) continue;

			expect(source, `${file} must import getErrorMessage`).toContain(
				'from "@/lib/errorMessage"',
			);
			expect(source, `${file} must call getErrorMessage`).toMatch(
				/\bgetErrorMessage\s*\(/,
			);
			expect(source, `${file} must not use String(error)`).not.toMatch(
				LOCAL_STRING_EXTRACTION,
			);
			expect(
				source,
				`${file} must not extract Error.message locally`,
			).not.toMatch(LOCAL_ERROR_MESSAGE_TERNARY);
			expect(source, `${file} must not declare a local helper`).not.toMatch(
				LOCAL_HELPER_DECLARATION,
			);
		}
	});

	it("production sourceへ局所抽出を再導入しない", () => {
		for (const [file, source] of Object.entries(TYPESCRIPT_SOURCES)) {
			if (file.includes(".test.") || file.endsWith("/errorMessage.ts")) {
				continue;
			}

			expect(source, `${file} must not use String(error)`).not.toMatch(
				LOCAL_STRING_EXTRACTION,
			);
			expect(
				source,
				`${file} must not extract Error.message locally`,
			).not.toMatch(LOCAL_ERROR_MESSAGE_TERNARY);
			expect(source, `${file} must not declare a local helper`).not.toMatch(
				LOCAL_HELPER_DECLARATION,
			);
		}
	});
});
