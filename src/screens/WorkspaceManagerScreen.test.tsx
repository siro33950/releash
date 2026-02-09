import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceManagerScreen } from "./WorkspaceManagerScreen";

describe("WorkspaceManagerScreen", () => {
	it("renders no-repo message when repoPath is null", () => {
		render(
			<WorkspaceManagerScreen repoPath={null} onSelectWorktree={vi.fn()} />,
		);
		expect(screen.getByText("No git repository detected")).toBeInTheDocument();
	});

	it("renders loading state then repo name when repoPath is set", () => {
		render(
			<WorkspaceManagerScreen
				repoPath="/home/user/my-repo"
				onSelectWorktree={vi.fn()}
			/>,
		);
		expect(screen.getByText("my-repo")).toBeInTheDocument();
	});
});
