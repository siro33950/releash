import type { MessageInitShape } from "@bufbuild/protobuf";
import {
	createConnectRouter,
	type HandlerContext,
	type ServiceImpl,
} from "@connectrpc/connect";
import {
	createFetchHandler,
	createLinkedAbortController,
} from "@connectrpc/connect/protocol";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import {
	type AttachTerminalSurfaceRequest,
	ClientService,
	type TerminalEventSchema,
	type TerminalSubscriptionEventSchema,
} from "@/generated/client_pb";

export function connectFixture(
	implementation: Partial<ServiceImpl<typeof ClientService>> & {
		terminalOutput?: (
			request: AttachTerminalSurfaceRequest,
			context: HandlerContext,
		) => AsyncIterable<MessageInitShape<typeof TerminalEventSchema>>;
	} = {},
) {
	const requests: Request[] = [];
	const subscriptions = new Map<
		string,
		{
			controller: ReadableStreamDefaultController<
				MessageInitShape<typeof TerminalSubscriptionEventSchema>
			>;
			signal: AbortSignal;
			close: () => void;
		}
	>();
	const attachments = new Map<string, AbortController>();
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
		async *subscribeTerminalSurfaces(request, context) {
			let controller!: ReadableStreamDefaultController<
				MessageInitShape<typeof TerminalSubscriptionEventSchema>
			>;
			const stream = new ReadableStream<
				MessageInitShape<typeof TerminalSubscriptionEventSchema>
			>({
				start(value) {
					controller = value;
				},
			});
			const stop = () => {
				context.signal.removeEventListener("abort", stop);
				controller.close();
			};
			subscriptions.set(request.subscriptionId, {
				controller,
				signal: context.signal,
				close: stop,
			});
			context.signal.addEventListener("abort", stop, { once: true });
			try {
				yield { event: { case: "ready", value: {} } };
				const reader = stream.getReader();
				for (;;) {
					const next = await reader.read();
					if (next.done) break;
					yield next.value;
				}
			} finally {
				subscriptions.delete(request.subscriptionId);
				context.signal.removeEventListener("abort", stop);
			}
		},
		async attachTerminalSurface(request, context) {
			const subscription = subscriptions.get(request.subscriptionId);
			if (!subscription || !request.request || !implementation.terminalOutput)
				throw new Error("Missing terminal subscription or output fixture");
			const attachmentId = request.request.attachmentId ?? "";
			const abort = new AbortController();
			attachments.get(attachmentId)?.abort();
			attachments.set(attachmentId, abort);
			const signal = createLinkedAbortController(
				abort.signal,
				subscription.signal,
			).signal;
			const output = implementation
				.terminalOutput(request.request, { ...context, signal })
				[Symbol.asyncIterator]();
			const initial = await output.next();
			const send = (
				event: MessageInitShape<
					typeof TerminalSubscriptionEventSchema
				>["event"],
			) => {
				if (!subscription.signal.aborted)
					subscription.controller.enqueue({
						attachmentId,
						streamId: request.streamId,
						event,
					});
			};
			let resynchronize = true;
			const sendItem = (item: MessageInitShape<typeof TerminalEventSchema>) => {
				if (
					item.item?.case === "exit" ||
					(item.item?.case === "snapshot" && item.item.value.isExited)
				)
					resynchronize = false;
				send({ case: "item", value: item });
			};
			if (!initial.done) sendItem(initial.value);
			void (async () => {
				try {
					if (!initial.done)
						for (;;) {
							const next = await output.next();
							if (next.done) break;
							sendItem(next.value);
						}
				} finally {
					if (attachments.get(attachmentId) === abort)
						attachments.delete(attachmentId);
					send({ case: "closed", value: { resynchronize } });
				}
			})().catch(() => {});
			return {};
		},
		detachTerminalSurface(request) {
			attachments.get(request.attachmentId ?? "")?.abort();
			return {};
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
		closeTerminalSubscriptions: () => {
			for (const subscription of subscriptions.values()) subscription.close();
		},
	};
}
