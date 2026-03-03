import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RemotePanel } from "./RemotePanel";

const defaultRemoteServer = {
	running: false,
	qrData: null,
	error: null,
	config: { port: 8080, token: "test-token" },
	interfaces: [{ name: "utun0", ip: "100.64.0.1", kind: "vpn" as const }],
	selectedIp: "100.64.0.1",
	setSelectedIp: vi.fn(),
	boundIp: null,
	connectionMode: null,
	showLanConfirm: false,
	startServer: vi.fn(),
	stopServer: vi.fn(),
	confirmLanStart: vi.fn(),
	cancelLanStart: vi.fn(),
	refreshQr: vi.fn(),
	refreshStatus: vi.fn(),
	updatePort: vi.fn(),
	regenerateToken: vi.fn(),
	updateTerminalStartupCommand: vi.fn(),
};

let mockRemoteServer = { ...defaultRemoteServer };
let mockRepoPaths: string[] = ["/repo/my-app"];

vi.mock("@/hooks/useRemoteServer", () => ({
	useRemoteServer: () => mockRemoteServer,
}));

vi.mock("@/hooks/useRepoList", () => ({
	useRepoList: () => ({ repoPaths: mockRepoPaths }),
}));

describe("RemotePanel", () => {
	beforeEach(() => {
		mockRemoteServer = { ...defaultRemoteServer };
		mockRepoPaths = ["/repo/my-app"];
	});

	it("should show warning when no repos are open", () => {
		mockRepoPaths = [];

		render(<RemotePanel terminalStartupCommand="" />);

		expect(
			screen.getByText("Open a folder before starting server"),
		).toBeInTheDocument();
	});

	it("should disable Start Server button when no repos are open", () => {
		mockRepoPaths = [];

		render(<RemotePanel terminalStartupCommand="" />);

		const button = screen.getByRole("button", { name: "Start Server" });
		expect(button).toBeDisabled();
	});

	it("should show Start Server button when server is stopped", () => {
		render(<RemotePanel terminalStartupCommand="" />);

		expect(
			screen.getByRole("button", { name: "Start Server" }),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Stop Server" }),
		).not.toBeInTheDocument();
	});

	it("should show Stop Server button when server is running", () => {
		mockRemoteServer = { ...defaultRemoteServer, running: true };

		render(<RemotePanel terminalStartupCommand="" />);

		expect(
			screen.getByRole("button", { name: "Stop Server" }),
		).toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Start Server" }),
		).not.toBeInTheDocument();
	});
});
