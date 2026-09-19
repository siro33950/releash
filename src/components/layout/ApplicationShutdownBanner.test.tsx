import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApplicationShutdownBanner } from "./ApplicationShutdownBanner";

const { mockInvoke } = vi.hoisted(() => ({ mockInvoke: vi.fn() }));

vi.mock("@/lib/client", () => ({
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
}));

describe("ApplicationShutdownBanner", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		globalThis.localStorage.clear();
	});

	it("renders the durable quit identity and intent when shutdown outcome is unknown", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_application_attempts":
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

		render(<ApplicationShutdownBanner container={document.body} />);

		const warning = await screen.findByTestId("shutdown-outcome-unknown");
		expect(warning).toHaveTextContent("Application shutdown outcome unknown");
		expect(warning).toHaveTextContent("quit-unknown-42");
		expect(warning).toHaveTextContent("restart (42)");
	});

	it("stays out of the way while no quit flight exists", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_application_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({ type: "current", plan: null });
				default:
					return Promise.resolve(null);
			}
		});

		render(<ApplicationShutdownBanner container={document.body} />);

		await vi.waitFor(() =>
			expect(
				mockInvoke.mock.calls.some(
					([command]) => command === "get_application_shutdown",
				),
			).toBe(true),
		);
		expect(screen.queryByTestId("application-shutdown")).toBeNull();
	});
	it("表示先を切り替えても終了監督を再作成しない", async () => {
		mockInvoke.mockImplementation(async (command) =>
			command === "list_pending_application_attempts"
				? { entries: [], next_cursor: null }
				: {
						type: "outcome_unknown",
						operation_id: "pending",
						intent: { type: "restart", code: 0 },
					},
		);
		const first = document.createElement("div");
		const second = document.createElement("div");
		document.body.append(first, second);
		const view = render(<ApplicationShutdownBanner container={first} />);
		await screen.findByTestId("shutdown-outcome-unknown");
		view.rerender(<ApplicationShutdownBanner container={second} />);
		expect(first).toBeEmptyDOMElement();
		expect(second).toHaveTextContent("pending");
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "get_application_shutdown",
			),
		).toHaveLength(1);
		view.unmount();
		first.remove();
		second.remove();
	});
});
