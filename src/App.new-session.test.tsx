import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	openWorktreeTab: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn((command: string) => {
		if (command === "get_cwd" || command === "get_main_repo_path") {
			return Promise.resolve("/repo");
		}
		if (command === "list_worktrees") return Promise.resolve([]);
		return Promise.resolve(null);
	}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/hooks/useSettings", () => ({
	useSettings: () => ({
		settings: { autoUpdate: false, theme: "dark" },
		updateSettings: vi.fn(),
		updateTheme: vi.fn(),
	}),
}));
vi.mock("@/hooks/useUpdateChecker", () => ({
	useUpdateChecker: () => null,
}));
vi.mock("@/hooks/useWorkspaceNavigation", () => ({
	useWorkspaceNavigation: () => ({
		worktrees: [{ id: "wt", rootPath: "/wt" }],
		selectedWorktreeId: "wt",
		openWorktreeTab: mocks.openWorktreeTab,
	}),
}));
vi.mock("@/hooks/useRepoList", () => ({
	useRepoList: () => ({
		repoPaths: ["/repo"],
		addRepo: vi.fn(),
		removeRepo: vi.fn(),
		initFromCwd: vi.fn(),
	}),
}));
vi.mock("@/hooks/useMenuEvents", () => ({ useMenuEvents: vi.fn() }));
vi.mock("@/components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: () => null,
}));
vi.mock("@/components/workspace/WorkspaceList", () => ({
	WorkspaceList: ({
		onCreateSession,
	}: {
		onCreateSession: (worktreePath: string) => void;
	}) => (
		<button type="button" onClick={() => onCreateSession("/wt")}>
			Create session
		</button>
	),
}));
vi.mock("@/screens/MainLayout", () => ({
	MainLayout: ({
		leftNav,
		newSessionCreationRequest,
		onCenterSelectionResolved,
	}: {
		leftNav: React.ReactNode;
		newSessionCreationRequest?: { requestId: number } | null;
		onCenterSelectionResolved?: (selection: {
			kind: "node";
			worktreePath: string;
			nodeId: string;
		}) => void;
	}) => (
		<div>
			{leftNav}
			<div data-testid="request-id">
				{newSessionCreationRequest?.requestId ?? "none"}
			</div>
			<button
				type="button"
				onClick={() =>
					onCenterSelectionResolved?.({
						kind: "node",
						worktreePath: "/wt",
						nodeId: "node-created",
					})
				}
			>
				Resolve request
			</button>
		</div>
	),
}));

const { default: App } = await import("./App");

describe("App NewSession creation requests", () => {
	it("uses a monotonic id across two successful consecutive creations", () => {
		render(<App />);

		fireEvent.click(screen.getByRole("button", { name: "Create session" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent("1");
		fireEvent.click(screen.getByRole("button", { name: "Resolve request" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent("none");

		fireEvent.click(screen.getByRole("button", { name: "Create session" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent("2");
		expect(mocks.openWorktreeTab).toHaveBeenCalledTimes(2);
	});
});
