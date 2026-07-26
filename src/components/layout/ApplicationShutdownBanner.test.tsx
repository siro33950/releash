import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApplicationShutdownBanner } from "./ApplicationShutdownBanner";

const { mockInvoke } = vi.hoisted(() => ({ mockInvoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@/hooks/useSessionStore", () => ({
	getAcceptedSendOperation: vi.fn().mockResolvedValue(null),
	listAcceptedPermissionResponseOperations: vi.fn().mockResolvedValue([]),
	redispatchPendingLifecycleAttempts: vi.fn().mockResolvedValue(undefined),
	redispatchPendingPermissionResponseAttempts: vi
		.fn()
		.mockResolvedValue(undefined),
	redispatchPendingSendAttempts: vi.fn().mockResolvedValue(undefined),
	redispatchPendingStopAttempts: vi.fn().mockResolvedValue(undefined),
}));

describe("ApplicationShutdownBanner", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		globalThis.localStorage.clear();
	});

	it("renders the durable quit identity and intent when shutdown outcome is unknown", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({
						type: "outcome_unknown",
						operation_id: "quit-unknown-42",
						intent: { type: "restart", code: 42 },
					});
				default:
					return Promise.resolve(null);
			}
		});

		render(<ApplicationShutdownBanner />);

		const warning = await screen.findByTestId("shutdown-outcome-unknown");
		expect(warning).toHaveTextContent("Application shutdown outcome unknown");
		expect(warning).toHaveTextContent("quit-unknown-42");
		expect(warning).toHaveTextContent("restart (42)");
	});

	it("stays out of the way while no quit flight exists", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({ type: "current", plan: null });
				default:
					return Promise.resolve(null);
			}
		});

		render(<ApplicationShutdownBanner />);

		await vi.waitFor(() =>
			expect(
				mockInvoke.mock.calls.some(
					([command]) => command === "get_application_shutdown",
				),
			).toBe(true),
		);
		expect(screen.queryByTestId("application-shutdown")).toBeNull();
	});
});
