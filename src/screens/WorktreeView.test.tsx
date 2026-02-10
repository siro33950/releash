import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "@/types/settings";
import { WorktreeView } from "./WorktreeView";

vi.mock("@/hooks/useCurrentBranch", () => ({
	useCurrentBranch: () => ({ branch: "main", refresh: vi.fn() }),
}));

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="panel-group">{children}</div>
	),
	Panel: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="panel">{children}</div>
	),
	Separator: () => <div data-testid="separator" />,
}));

describe("WorktreeView", () => {
	it("renders without crashing", () => {
		render(
			<WorktreeView
				rootPath="/test/path"
				settings={DEFAULT_SETTINGS}
				updateTheme={vi.fn()}
				updateFontSize={vi.fn()}
				updateDefaultDiffBase={vi.fn()}
				updateDefaultDiffMode={vi.fn()}
				updateTerminalStartupCommand={vi.fn()}
				onGoHome={vi.fn()}
			/>,
		);
		expect(screen.getByText("No file selected")).toBeInTheDocument();
	});
});
