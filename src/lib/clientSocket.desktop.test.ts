import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { type Channel, invoke } from "@tauri-apps/api/core";
import { afterEach, expect, it, vi } from "vitest";
import { EnvelopeSchema } from "@/generated/client_pb";
import { getClientStatus, invokeClient } from "./clientSocket";

vi.unmock("./clientSocket");
afterEach(() => window.dispatchEvent(new Event("pagehide")));

it.each(["not_sent", "unknown"] as const)(
	"desktop境界の非同期%sを未送信と結果不明へ区別する",
	async (state) => {
		let channel: Channel<number[] | null>;
		let rejectWrite: (reason: unknown) => void = () => {
			throw new Error("write not started");
		};
		let writes = 0;
		vi.mocked(invoke).mockImplementation(async (command, args) => {
			if (command === "get_client_endpoint")
				throw new Error("Removed endpoint RPC");
			if (command === "list_client_handoff") return [];
			if (command === "attach_desktop_client") {
				channel = (args as { channel: Channel<number[] | null> }).channel;
				return Array.from(
					toBinary(
						EnvelopeSchema,
						create(EnvelopeSchema, {
							body: {
								case: "hello",
								value: {
									instanceId: "instance",
									launchId: "launch",
									release: "release",
								},
							},
						}),
					),
				);
			}
			if (command === "send_desktop_client_frame") {
				const { body } = fromBinary(
					EnvelopeSchema,
					new Uint8Array((args as { bytes: number[] }).bytes),
				);
				if (body.case === "operationQuery") {
					channel.onmessage(
						Array.from(
							toBinary(
								EnvelopeSchema,
								create(EnvelopeSchema, {
									body: {
										case: "operationStatus",
										value: {
											requestId: body.value.requestId,
											queryId: body.value.queryId,
											state: "ready",
										},
									},
								}),
							),
						),
					);
				} else if (body.case === "request") {
					writes++;
					return new Promise((_, reject) => {
						rejectWrite = reject;
					});
				}
			}
		});
		const uncertain = vi.fn();
		const rejected = vi.fn();
		void invokeClient(
			"update_external_editor",
			{ editor: "vim" },
			{ onUncertain: uncertain },
		).catch(rejected);
		await vi.waitFor(() => expect(writes).toBe(1));
		rejectWrite({ state, message: "write failed asynchronously" });
		await vi.waitFor(() =>
			expect(getClientStatus().operations).toContainEqual(
				expect.objectContaining({ command: "update_external_editor", state }),
			),
		);
		if (state === "not_sent") {
			expect(rejected).toHaveBeenCalledWith(
				expect.objectContaining({ state: "not_sent" }),
			);
			expect(uncertain).not.toHaveBeenCalled();
		} else {
			expect(uncertain).toHaveBeenCalledWith(
				expect.objectContaining({ state: "unknown" }),
			);
			expect(rejected).not.toHaveBeenCalled();
		}
		expect(writes).toBe(1);
		expect(invoke).not.toHaveBeenCalledWith("get_client_endpoint");
	},
);
