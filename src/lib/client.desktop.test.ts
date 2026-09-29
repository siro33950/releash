import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.unmock("@/lib/client");
let invoke: typeof import("@tauri-apps/api/core").invoke;
let connectFixture: typeof import("@/test/connect").connectFixture;
let completeClientRestoration: typeof import("./client").completeClientRestoration;
let invokeClient: typeof import("./client").invokeClient;
beforeEach(async () => {
	vi.resetModules();
	({ invoke } = await import("@tauri-apps/api/core"));
	({ connectFixture } = await import("@/test/connect"));
	({ completeClientRestoration, invokeClient } = await import("./client"));
});
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	vi.unstubAllGlobals();
});

it("desktopの業務要求をHTTPへ送りIPCには接続情報と復元完了だけを渡す", async () => {
	vi.mocked(invoke).mockClear();
	const fixture = connectFixture({
		addRepoPath: () => ({ value: true }),
	});
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	await completeClientRestoration(7);
	for (const request of fixture.requests)
		expect(request.headers.get("authorization")).toBe("Bearer client-token");
	expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).toEqual([
		"get_client_endpoint",
		"validate_daemon_connection",
		"complete_desktop_restoration",
	]);
	expect(invoke).toHaveBeenLastCalledWith("complete_desktop_restoration", {
		launchId: "launch",
		attachmentId: expect.any(String),
		generation: 7,
	});
});

it("破棄済み画面の遅い接続情報が次の接続を上書きしない", async () => {
	connectFixture({ addRepoPath: () => ({ value: true }) });
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing endpoint fixture");
	let release!: (value: unknown) => void;
	let endpoints = 0;
	vi.mocked(invoke).mockImplementation((command, args) => {
		if (command === "get_client_endpoint" && ++endpoints === 1)
			return new Promise((resolve) => {
				release = resolve;
			});
		return original(command, args);
	});
	const old = invokeClient("add_repo_path", { path: "/repo" }).catch(
		(error) => error,
	);
	window.dispatchEvent(
		new PageTransitionEvent("pagehide", { persisted: true }),
	);
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	release({ url: "http://127.0.0.1:9829", token: "old", launchId: "launch" });
	expect(await old).toBeInstanceOf(Error);
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	expect(endpoints).toBe(2);
});

it("Rustの同一性検証が失敗した接続では業務RPCも復元完了も呼ばない", async () => {
	const read = vi.fn(() => ({ value: true }));
	connectFixture({
		getServerInfo: () => ({ launchId: "different", release: "test" }),
		addRepoPath: read,
	});
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing fixture implementation");
	vi.mocked(invoke).mockClear();
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "validate_daemon_connection")
			throw new Error("Daemon identity changed");
		return original(command, args);
	});
	await expect(
		invokeClient("add_repo_path", { path: "/repo" }),
	).rejects.toThrow("Daemon identity changed");
	await expect(completeClientRestoration(7)).rejects.toThrow(
		"Daemon connection is TRANSIENT_FAILURE",
	);
	expect(invoke).toHaveBeenCalledWith("validate_daemon_connection", {
		launchId: "different",
		release: "test",
	});
	expect(read).not.toHaveBeenCalled();
	expect(
		vi
			.mocked(invoke)
			.mock.calls.some(([name]) => name === "complete_desktop_restoration"),
	).toBe(false);
});
