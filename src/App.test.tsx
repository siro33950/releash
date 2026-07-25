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
	localStorage.clear();
	mockInvoke.mockImplementation((cmd: string) => {
		if (cmd === "get_application_startup_outcome") {
			return Promise.resolve({ type: "ready" });
		}
		if (cmd === "get_performance_telemetry_enabled") {
			return Promise.resolve(true);
		}
		return Promise.reject(new Error("not in a git repo"));
	});
});

describe("App", () => {
	it("renders layout with empty state message", async () => {
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		await waitFor(() => {
			expect(
				screen.getByText("Select a worktree from the sidebar to start working"),
			).toBeInTheDocument();
		});
	});

	it("reads performance telemetry from Rust without writing localStorage to Rust on startup", async () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: true }),
		);
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_application_startup_outcome") {
				return Promise.resolve({ type: "ready" });
			}
			if (cmd === "get_performance_telemetry_enabled") {
				return Promise.resolve(false);
			}
			return Promise.reject(new Error("not in a git repo"));
		});

		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_performance_telemetry_enabled",
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"update_performance_telemetry",
			{ enabled: true },
		);
	});
});
