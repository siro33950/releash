import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRemoteServer } from "./useRemoteServer";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

type ListenCallback = (event: { payload: Record<string, unknown> }) => void;
let capturedListeners: Map<string, ListenCallback>;

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn((eventName: string, callback: ListenCallback) => {
		capturedListeners.set(eventName, callback);
		return Promise.resolve(() => {
			capturedListeners.delete(eventName);
		});
	}),
}));

vi.mock("@/lib/telemetry", () => ({
	trackEvent: vi.fn(),
}));

const defaultConfig = { port: 8080, token: "test-token" };
const defaultInterfaces = [
	{ name: "utun0", ip: "100.64.0.1", kind: "vpn" },
	{ name: "en0", ip: "192.168.1.10", kind: "lan" },
];
const defaultServerInfo = {
	running: false,
	bound_ip: null,
	connection_mode: null,
};

function setupDefaultMocks() {
	mockInvoke.mockImplementation((cmd: string) => {
		switch (cmd) {
			case "get_server_config":
				return Promise.resolve(defaultConfig);
			case "get_network_info":
				return Promise.resolve(defaultInterfaces);
			case "get_server_info":
				return Promise.resolve(defaultServerInfo);
			case "get_connection_qr":
				return Promise.resolve({
					url: "http://100.64.0.1:8080",
					svg: "<svg/>",
					token_svg: "<svg/>",
				});
			default:
				return Promise.resolve(null);
		}
	});
}

describe("useRemoteServer", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		capturedListeners = new Map();
		setupDefaultMocks();
	});

	it("should call get_server_config, get_network_info, get_server_info on mount", async () => {
		renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_server_config");
			expect(mockInvoke).toHaveBeenCalledWith("get_network_info");
			expect(mockInvoke).toHaveBeenCalledWith("get_server_info");
		});
	});

	it("should set error when selectedIp is null and startServer is called", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve([]);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_network_info");
		});

		await act(async () => {
			await result.current.startServer();
		});

		expect(result.current.error).toBe("Please select an IP address");
	});

	it("should invoke start_server when VPN IP is selected", async () => {
		mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve(defaultInterfaces);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				case "start_server":
					return Promise.resolve({
						ip: (args as { bindIp: string }).bindIp,
						mode: "vpn",
					});
				case "get_connection_qr":
					return Promise.resolve({
						url: "http://100.64.0.1:8080",
						svg: "<svg/>",
						token_svg: "<svg/>",
					});
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.selectedIp).toBe("100.64.0.1");
		});

		await act(async () => {
			await result.current.startServer();
		});

		expect(mockInvoke).toHaveBeenCalledWith("start_server", {
			bindIp: "100.64.0.1",
		});
		expect(result.current.running).toBe(true);
		expect(result.current.boundIp).toBe("100.64.0.1");
		expect(result.current.connectionMode).toBe("vpn");
	});

	it("should show LAN confirm when LAN IP is selected", async () => {
		const lanOnly = [{ name: "en0", ip: "192.168.1.10", kind: "lan" }];

		mockInvoke.mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve(lanOnly);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.selectedIp).toBe("192.168.1.10");
		});

		await act(async () => {
			await result.current.startServer();
		});

		expect(result.current.showLanConfirm).toBe(true);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_server",
			expect.anything(),
		);
	});

	it("should invoke start_server on confirmLanStart and hide dialog", async () => {
		const lanOnly = [{ name: "en0", ip: "192.168.1.10", kind: "lan" }];

		mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve(lanOnly);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				case "start_server":
					return Promise.resolve({
						ip: (args as { bindIp: string }).bindIp,
						mode: "lan",
					});
				case "get_connection_qr":
					return Promise.resolve({
						url: "http://192.168.1.10:8080",
						svg: "<svg/>",
						token_svg: "<svg/>",
					});
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.selectedIp).toBe("192.168.1.10");
		});

		await act(async () => {
			await result.current.startServer();
		});

		expect(result.current.showLanConfirm).toBe(true);

		await act(async () => {
			await result.current.confirmLanStart();
		});

		expect(result.current.showLanConfirm).toBe(false);
		expect(mockInvoke).toHaveBeenCalledWith("start_server", {
			bindIp: "192.168.1.10",
		});
		expect(result.current.running).toBe(true);
	});

	it("should hide dialog on cancelLanStart without starting", async () => {
		const lanOnly = [{ name: "en0", ip: "192.168.1.10", kind: "lan" }];

		mockInvoke.mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve(lanOnly);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.selectedIp).toBe("192.168.1.10");
		});

		await act(async () => {
			await result.current.startServer();
		});

		act(() => {
			result.current.cancelLanStart();
		});

		expect(result.current.showLanConfirm).toBe(false);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_server",
			expect.anything(),
		);
	});

	it("should invoke stop_server and reset state on stopServer", async () => {
		mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.resolve(defaultConfig);
				case "get_network_info":
					return Promise.resolve(defaultInterfaces);
				case "get_server_info":
					return Promise.resolve({
						running: true,
						bound_ip: "100.64.0.1",
						connection_mode: "vpn",
					});
				case "start_server":
					return Promise.resolve({
						ip: (args as { bindIp: string }).bindIp,
						mode: "vpn",
					});
				case "get_connection_qr":
					return Promise.resolve({
						url: "http://100.64.0.1:8080",
						svg: "<svg/>",
						token_svg: "<svg/>",
					});
				case "stop_server":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.running).toBe(true);
		});

		await act(async () => {
			await result.current.stopServer();
		});

		expect(mockInvoke).toHaveBeenCalledWith("stop_server");
		expect(result.current.running).toBe(false);
		expect(result.current.qrData).toBeNull();
		expect(result.current.boundIp).toBeNull();
		expect(result.current.connectionMode).toBeNull();
	});

	it("should update running state when server-status-changed event fires", async () => {
		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(capturedListeners.has("server-status-changed")).toBe(true);
		});

		act(() => {
			const listener = capturedListeners.get("server-status-changed");
			listener?.({
				payload: {
					running: true,
					bound_ip: "100.64.0.1",
					connection_mode: "vpn",
				},
			});
		});

		expect(result.current.running).toBe(true);
		expect(result.current.boundIp).toBe("100.64.0.1");
		expect(result.current.connectionMode).toBe("vpn");
	});

	it("should clear state when server-status-changed event fires with running=false", async () => {
		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(capturedListeners.has("server-status-changed")).toBe(true);
		});

		act(() => {
			const listener = capturedListeners.get("server-status-changed");
			listener?.({
				payload: {
					running: true,
					bound_ip: "100.64.0.1",
					connection_mode: "vpn",
				},
			});
		});

		await vi.waitFor(() => {
			expect(result.current.running).toBe(true);
		});

		act(() => {
			const listener = capturedListeners.get("server-status-changed");
			listener?.({
				payload: {
					running: false,
					bound_ip: null,
					connection_mode: null,
				},
			});
		});

		expect(result.current.running).toBe(false);
		expect(result.current.boundIp).toBeNull();
		expect(result.current.connectionMode).toBeNull();
		expect(result.current.qrData).toBeNull();
	});

	it("should set error when invoke fails", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_server_config":
					return Promise.reject(new Error("config error"));
				case "get_network_info":
					return Promise.resolve(defaultInterfaces);
				case "get_server_info":
					return Promise.resolve(defaultServerInfo);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useRemoteServer());

		await vi.waitFor(() => {
			expect(result.current.error).toBeTruthy();
		});
	});
});
