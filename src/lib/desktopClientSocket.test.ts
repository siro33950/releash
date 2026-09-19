import { create, toBinary } from "@bufbuild/protobuf";
import { Channel, invoke } from "@tauri-apps/api/core";
import { beforeEach, expect, it, vi } from "vitest";
import { EnvelopeSchema } from "@/generated/client_pb";
import { DesktopClientSocket } from "./desktopClientSocket";

const hello = () =>
	Array.from(
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
const command = () =>
	toBinary(
		EnvelopeSchema,
		create(EnvelopeSchema, {
			body: {
				case: "request",
				value: {
					requestId: "request",
					command: { case: "getRepoPaths", value: {} },
				},
			},
		}),
	);
beforeEach(() => vi.mocked(invoke).mockReset());
it("Rust所有の接続へ購読し新しいWSを作らずフレームを渡す", async () => {
	vi.mocked(invoke).mockResolvedValue(hello());
	const browserSocket = vi.spyOn(globalThis, "WebSocket");
	const socket = new DesktopClientSocket();
	socket.onopen = vi.fn();
	socket.onmessage = vi.fn();
	await vi.waitFor(() => expect(socket.onopen).toHaveBeenCalledOnce());
	expect(browserSocket).not.toHaveBeenCalled();
	const args = vi.mocked(invoke).mock.calls[0][1] as Record<string, unknown>;
	expect(args?.channel).toBeInstanceOf(Channel);
	await socket.send(
		toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, { body: { case: "hello", value: {} } }),
		),
	);
	expect(socket.onmessage).toHaveBeenCalledWith({
		data: new Uint8Array(hello()).buffer,
	});
	await socket.send(command());
	expect(invoke).toHaveBeenCalledWith("send_desktop_client_frame", {
		launchId: "launch",
		bytes: Array.from(command()),
	});
	socket.close();
	expect(invoke).toHaveBeenCalledWith("detach_desktop_client", {
		attachmentId: args?.attachmentId,
	});
});
it.each(["not_sent", "unknown"])(
	"送信境界の%sを呼び出し元へ返す",
	async (state) => {
		vi.mocked(invoke).mockImplementation(async (name) => {
			if (name === "attach_desktop_client") return hello();
			if (name === "send_desktop_client_frame")
				throw { state, message: "transport failure" };
		});
		const socket = new DesktopClientSocket();
		socket.onopen = vi.fn();
		await vi.waitFor(() => expect(socket.onopen).toHaveBeenCalledOnce());
		await expect(socket.send(command())).rejects.toEqual({
			state,
			message: "transport failure",
		});
		socket.close();
	},
);
it("購読失敗と切断を表示側へ通知し破棄後のpushを渡さない", async () => {
	vi.mocked(invoke).mockRejectedValueOnce(new Error("not ready"));
	const unavailable = new DesktopClientSocket();
	unavailable.onerror = vi.fn();
	await vi.waitFor(() => expect(unavailable.onerror).toHaveBeenCalledOnce());
	vi.mocked(invoke).mockResolvedValue(hello());
	const socket = new DesktopClientSocket();
	socket.onopen = vi.fn();
	socket.onclose = vi.fn();
	socket.onmessage = vi.fn();
	await vi.waitFor(() => expect(socket.onopen).toHaveBeenCalledOnce());
	const channel = (
		vi.mocked(invoke).mock.calls[1][1] as Record<string, unknown>
	).channel as Channel<number[] | null>;
	channel.onmessage(null);
	channel.onmessage(hello());
	expect(socket.onclose).toHaveBeenCalledOnce();
	expect(socket.onmessage).not.toHaveBeenCalled();
});
