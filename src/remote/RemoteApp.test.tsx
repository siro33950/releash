import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteApp } from "./RemoteApp";

vi.mock("./hooks/useWebSocket", () => ({
	useWebSocket: vi.fn(() => ({
		status: "disconnected" as const,
		send: vi.fn(),
		disconnect: vi.fn(),
		reconnect: vi.fn(),
	})),
}));

vi.mock("./hooks/useRemoteWorktrees", () => ({
	useRemoteWorktrees: vi.fn(() => ({
		worktrees: [],
		loading: false,
		refresh: vi.fn(),
		select: vi.fn(),
	})),
}));

import { useWebSocket } from "./hooks/useWebSocket";

const mockUseWebSocket = vi.mocked(useWebSocket);

describe("RemoteApp", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockUseWebSocket.mockReturnValue({
			status: "disconnected",
			send: vi.fn(),
			disconnect: vi.fn(),
			reconnect: vi.fn(),
		});
	});

	it("shows connection form initially", () => {
		render(<RemoteApp />);
		expect(screen.getByText("接続")).toBeInTheDocument();
	});

	it("shows connection form with host input", () => {
		render(<RemoteApp />);
		expect(screen.getByLabelText(/ホスト/)).toBeInTheDocument();
	});

	it("shows token input", () => {
		render(<RemoteApp />);
		expect(screen.getByLabelText(/トークン/)).toBeInTheDocument();
	});

	it("transitions to main UI after connection", async () => {
		const user = userEvent.setup();
		mockUseWebSocket.mockReturnValue({
			status: "connected",
			send: vi.fn(),
			disconnect: vi.fn(),
			reconnect: vi.fn(),
		});

		render(<RemoteApp />);

		const hostInput = screen.getByLabelText(/ホスト/);
		const tokenInput = screen.getByLabelText(/トークン/);

		await user.clear(hostInput);
		await user.type(hostInput, "192.168.1.100:9700");
		await user.clear(tokenInput);
		await user.type(tokenInput, "mytoken");

		const connectBtn = screen.getByText("接続");
		await user.click(connectBtn);

		expect(screen.getByText("Releash Remote")).toBeInTheDocument();
		expect(screen.getByText("切断")).toBeInTheDocument();
	});
});
