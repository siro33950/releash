import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useRemoteAutoStart } from "./useRemoteAutoStart";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useRemoteAutoStart", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("should not start when ready is false", () => {
		renderHook(() => useRemoteAutoStart(["/repo"], false));
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should not start when repoPaths is empty", () => {
		renderHook(() => useRemoteAutoStart([], true));
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should not start when auto_start is false", async () => {
		mockInvoke.mockResolvedValueOnce({
			auto_start: false,
			auto_start_on_lan: false,
		});

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_remote_config");
		});
		expect(mockInvoke).not.toHaveBeenCalledWith("get_server_info");
	});

	it("should not start when server is already running", async () => {
		mockInvoke
			.mockResolvedValueOnce({ auto_start: true, auto_start_on_lan: false })
			.mockResolvedValueOnce({
				running: true,
				bound_ip: "10.0.0.1",
				connection_mode: "vpn",
			});

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_server_info");
		});
		expect(mockInvoke).not.toHaveBeenCalledWith("get_network_info");
	});

	it("should start with VPN IP when VPN interface is available", async () => {
		mockInvoke
			.mockResolvedValueOnce({ auto_start: true, auto_start_on_lan: false })
			.mockResolvedValueOnce({
				running: false,
				bound_ip: null,
				connection_mode: null,
			})
			.mockResolvedValueOnce([
				{ name: "utun0", ip: "10.8.0.2", kind: "vpn" },
				{ name: "en0", ip: "192.168.1.10", kind: "lan" },
			])
			.mockResolvedValueOnce({ ip: "10.8.0.2", mode: "vpn" });

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_server", {
				repoPaths: ["/repo"],
				bindIp: "10.8.0.2",
			});
		});
	});

	it("should start with LAN IP when no VPN and auto_start_on_lan is true", async () => {
		mockInvoke
			.mockResolvedValueOnce({ auto_start: true, auto_start_on_lan: true })
			.mockResolvedValueOnce({
				running: false,
				bound_ip: null,
				connection_mode: null,
			})
			.mockResolvedValueOnce([{ name: "en0", ip: "192.168.1.10", kind: "lan" }])
			.mockResolvedValueOnce({ ip: "192.168.1.10", mode: "lan" });

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_server", {
				repoPaths: ["/repo"],
				bindIp: "192.168.1.10",
			});
		});
	});

	it("should not start when no VPN and auto_start_on_lan is false", async () => {
		mockInvoke
			.mockResolvedValueOnce({ auto_start: true, auto_start_on_lan: false })
			.mockResolvedValueOnce({
				running: false,
				bound_ip: null,
				connection_mode: null,
			})
			.mockResolvedValueOnce([
				{ name: "en0", ip: "192.168.1.10", kind: "lan" },
			]);

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_network_info");
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_server",
			expect.anything(),
		);
	});

	it("should not start when no interfaces available", async () => {
		mockInvoke
			.mockResolvedValueOnce({ auto_start: true, auto_start_on_lan: true })
			.mockResolvedValueOnce({
				running: false,
				bound_ip: null,
				connection_mode: null,
			})
			.mockResolvedValueOnce([]);

		renderHook(() => useRemoteAutoStart(["/repo"], true));
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_network_info");
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"start_server",
			expect.anything(),
		);
	});

	it("should only attempt once even if re-rendered", async () => {
		mockInvoke.mockResolvedValueOnce({
			auto_start: false,
			auto_start_on_lan: false,
		});

		const { rerender } = renderHook(
			({ paths, ready }) => useRemoteAutoStart(paths, ready),
			{ initialProps: { paths: ["/repo"], ready: true } },
		);

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_remote_config");
		});

		mockInvoke.mockClear();
		rerender({ paths: ["/repo", "/repo2"], ready: true });

		await new Promise((r) => setTimeout(r, 50));
		expect(mockInvoke).not.toHaveBeenCalled();
	});
});
