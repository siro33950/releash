import { createConnectRouter, type ServiceImpl } from "@connectrpc/connect";
import { createFetchHandler } from "@connectrpc/connect/protocol";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import { ClientService } from "@/generated/client_pb";

export function connectFixture(
	implementation: Partial<ServiceImpl<typeof ClientService>> = {},
) {
	const requests: Request[] = [];
	const router = createConnectRouter().service(ClientService, {
		getServerInfo: () => ({
			launchId: "launch",
			release: "test",
		}),
		async *subscribePush(_, context) {
			yield { event: { case: "resync" as const, value: {} } };
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
		...implementation,
	});
	const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
		const request = new Request(input, init);
		requests.push(request);
		const handler = router.handlers.find(
			(handler) => handler.requestPath === new URL(request.url).pathname,
		);
		if (!handler) throw new Error(`Unexpected RPC: ${request.url}`);
		request.signal.throwIfAborted();
		let onAbort!: () => void;
		const aborted = new Promise<never>((_, reject) => {
			onAbort = () => reject(request.signal.reason);
			request.signal.addEventListener("abort", onAbort, { once: true });
		});
		try {
			return await Promise.race([
				createFetchHandler(handler)(request),
				aborted,
			]);
		} finally {
			request.signal.removeEventListener("abort", onAbort);
		}
	});
	vi.stubGlobal("fetch", fetch);
	vi.mocked(invoke).mockImplementation(async (command) => {
		if (command === "get_client_endpoint")
			return {
				url: "http://127.0.0.1:9829",
				token: "client-token",
				launchId: "launch",
			};
		if (
			command === "validate_daemon_connection" ||
			command === "complete_desktop_restoration" ||
			command === "apply_desktop_settings"
		)
			return;
		throw new Error(`Unexpected IPC: ${command}`);
	});
	return {
		fetch,
		requests,
	};
}
