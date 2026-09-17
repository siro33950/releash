import { Channel, invoke } from "@tauri-apps/api/core";
import { decodeClientEnvelope } from "./clientProtocol";

export class DesktopClientSocket {
	onopen: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
	private closed = false;
	private launchId = "";
	private hello: number[] = [];
	private writes = Promise.resolve();
	private readonly attachmentId = crypto.randomUUID();

	constructor() {
		const channel = new Channel<number[] | null>();
		channel.onmessage = (bytes) => {
			if (bytes === null) this.close();
			else this.receive(bytes);
		};
		void invoke<number[]>("attach_desktop_client", {
			channel,
			attachmentId: this.attachmentId,
		})
			.then((hello) => {
				if (this.closed) return;
				this.hello = hello;
				const { body } = decodeClientEnvelope(new Uint8Array(hello).buffer);
				if (body.case !== "hello")
					throw new Error("Missing desktop client hello");
				this.launchId = body.value.launchId;
				this.onopen?.();
			})
			.catch(() => this.onerror?.());
	}
	private receive(bytes: number[]) {
		if (!this.closed) this.onmessage?.({ data: new Uint8Array(bytes).buffer });
	}
	send(bytes: Uint8Array): Promise<void> {
		if (this.closed)
			throw { state: "not_sent", message: "Desktop client is closed" };
		if (
			decodeClientEnvelope(new Uint8Array(bytes).buffer).body.case === "hello"
		) {
			this.receive(this.hello);
			return Promise.resolve();
		}
		const write = this.writes.then(() =>
			invoke<void>("send_desktop_client_frame", {
				bytes: Array.from(bytes),
				launchId: this.launchId,
			}),
		);
		this.writes = write.catch(() => {});
		if (
			decodeClientEnvelope(new Uint8Array(bytes).buffer).body.case === "request"
		)
			return write;
		return write.catch(() => {
			this.onerror?.();
		});
	}
	async completeRestoration(generation: number): Promise<void> {
		await this.writes;
		await invoke("complete_desktop_restoration", {
			launchId: this.launchId,
			attachmentId: this.attachmentId,
			generation,
		});
	}
	close() {
		if (this.closed) return;
		this.closed = true;
		void invoke("detach_desktop_client", {
			attachmentId: this.attachmentId,
		}).catch(() => {});
		this.onclose?.();
	}
}
