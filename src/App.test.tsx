import { invoke } from "@tauri-apps/api/core";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import App from "./App";

vi.mock("react-resizable-panels", () => {
	const Panel = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel">{children}</div>
	);
	const Group = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel-group">{children}</div>
	);
	const Separator = () => <div data-testid="separator" />;
	return { Panel, Group, Separator };
});

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
	mockInvoke.mockRejectedValue(new Error("not in a git repo"));
});

describe("App", () => {
	it("renders 3-column layout with empty state", async () => {
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		await waitFor(() => {
			expect(screen.getByText("No worktree selected")).toBeInTheDocument();
		});
	});
});
