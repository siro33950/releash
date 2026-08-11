import { describe, expect, it, vi } from "vitest";
import {
	createMessageChannelContinuation,
	type TerminalOutputContinuation,
	TerminalOutputScheduler,
	terminalOutputChunkLength,
} from "./terminalOutputScheduler";

class ManualContinuation implements TerminalOutputContinuation {
	readonly tasks: Array<() => void> = [];

	post(task: () => void): void {
		this.tasks.push(task);
	}

	runNext(): void {
		this.tasks.shift()?.();
	}

	dispose(): void {
		this.tasks.length = 0;
	}
}

describe("TerminalOutputScheduler", () => {
	it("surrogate pair直前に残り1 code unitでも前進不能loopへ入らない", () => {
		const data = `${"a".repeat(16 * 1024 - 1)}🙂`;

		expect(terminalOutputChunkLength(data, 0, 16 * 1024)).toBe(16 * 1024 - 1);
		expect(terminalOutputChunkLength(data, 16 * 1024 - 1, 1)).toBe(0);

		const writes: string[] = [];
		const callbacks: Array<() => void> = [];
		const scheduler = new TerminalOutputScheduler({
			write: (chunk, parsed) => {
				writes.push(chunk);
				callbacks.push(parsed);
			},
			continuation: new ManualContinuation(),
			clock: () => 0,
		});
		scheduler.enqueue(data);
		callbacks.shift()?.();
		callbacks.shift()?.();

		expect(writes).toEqual(["a".repeat(16 * 1024 - 1), "🙂"]);
		expect(scheduler.metrics().currentQueuedCodeUnits).toBe(0);
	});

	it("xterm parse callback前に次のchunkをwriteしない", () => {
		const callbacks: Array<() => void> = [];
		const write = vi.fn((_data: string, parsed: () => void) => {
			callbacks.push(parsed);
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation: new ManualContinuation(),
			clock: () => 0,
		});

		scheduler.enqueue("a".repeat(16 * 1024 + 1));

		expect(write).toHaveBeenCalledTimes(1);
		callbacks.shift()?.();
		expect(write).toHaveBeenCalledTimes(2);
	});

	it("一drainを8 writeでyieldする", () => {
		const continuation = new ManualContinuation();
		const write = vi.fn((_data: string, parsed: () => void) => parsed());
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation,
			clock: () => 0,
		});

		scheduler.enqueue("a".repeat(16 * 1024 * 9));

		expect(write).toHaveBeenCalledTimes(8);
		expect(continuation.tasks).toHaveLength(1);
		continuation.runNext();
		expect(write).toHaveBeenCalledTimes(9);
	});

	it("一drainを8msでyieldする", () => {
		const continuation = new ManualContinuation();
		let now = 0;
		const write = vi.fn((_data: string, parsed: () => void) => {
			now += 5;
			parsed();
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation,
			clock: () => now,
		});

		scheduler.enqueue("a".repeat(16 * 1024 * 3));

		expect(write).toHaveBeenCalledTimes(2);
		expect(continuation.tasks).toHaveLength(1);
	});

	it("UTF-16 code unitのcurrentとpeakをparse完了時まで保持する", () => {
		const callbacks: Array<() => void> = [];
		const scheduler = new TerminalOutputScheduler({
			write: (_data, parsed) => callbacks.push(parsed),
			continuation: new ManualContinuation(),
			clock: () => 0,
		});

		scheduler.enqueue("🙂a");
		expect(scheduler.metrics()).toMatchObject({
			currentQueuedCodeUnits: 3,
			peakQueuedCodeUnits: 3,
		});
		callbacks.shift()?.();
		expect(scheduler.metrics().currentQueuedCodeUnits).toBe(0);
	});

	it("drainは全parse完了後だけresolveする", async () => {
		let parsed!: () => void;
		const scheduler = new TerminalOutputScheduler({
			write: (_data, callback) => {
				parsed = callback;
			},
			continuation: new ManualContinuation(),
			clock: () => 0,
		});
		const resolved = vi.fn();

		scheduler.enqueue("pending");
		void scheduler.drain().then(resolved);
		await Promise.resolve();
		expect(resolved).not.toHaveBeenCalled();
		parsed();
		await Promise.resolve();
		expect(resolved).toHaveBeenCalledTimes(1);
	});

	it("coalesceした各outputのACKを対応する全dataのparse後に累積順で返す", () => {
		let parsed!: () => void;
		const acknowledgements: number[] = [];
		const scheduler = new TerminalOutputScheduler({
			write: (_data, callback) => {
				parsed = callback;
			},
			continuation: new ManualContinuation(),
			clock: () => 0,
		});

		scheduler.enqueue("first", () => acknowledgements.push(4));
		scheduler.enqueue("second", () => acknowledgements.push(5));

		expect(acknowledgements).toEqual([]);
		parsed();
		expect(acknowledgements).toEqual([4]);
		parsed();
		expect(acknowledgements).toEqual([4, 5]);
	});

	it("queue上限超過時は中間deltaを破棄し一度だけsnapshot再同期を要求する", () => {
		const callbacks: Array<() => void> = [];
		const onOverflow = vi.fn();
		const scheduler = new TerminalOutputScheduler({
			write: (_data, parsed) => callbacks.push(parsed),
			continuation: new ManualContinuation(),
			clock: () => 0,
			onOverflow,
		});

		scheduler.enqueue("a".repeat(2 * 1024 * 1024));
		scheduler.enqueue("overflow");
		scheduler.enqueue("ignored-during-resync");

		expect(onOverflow).toHaveBeenCalledTimes(1);
		expect(scheduler.metrics()).toMatchObject({
			currentQueuedCodeUnits: 16 * 1024,
			peakQueuedCodeUnits: 2 * 1024 * 1024,
			droppedBacklogs: 1,
			snapshotResyncs: 0,
		});

		callbacks.shift()?.();
		scheduler.resumeAfterSnapshot();
		scheduler.enqueue("after-snapshot");
		expect(scheduler.metrics().snapshotResyncs).toBe(1);
		expect(callbacks).toHaveLength(1);
	});
});

describe("MessageChannel continuation", () => {
	it("zero-delay継続にtimerを使わない", () => {
		const posted: unknown[] = [];
		const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");
		class FakeMessageChannel {
			port1 = { onmessage: null as null | (() => void), close: vi.fn() };
			port2 = {
				postMessage: (message: unknown) => posted.push(message),
				close: vi.fn(),
			};
		}
		vi.stubGlobal("MessageChannel", FakeMessageChannel);
		const continuation = createMessageChannelContinuation();

		continuation.post(vi.fn());

		expect(posted).toEqual([null]);
		expect(setTimeoutSpy).not.toHaveBeenCalled();
		continuation.dispose();
		vi.unstubAllGlobals();
	});
});

describe("TerminalOutputScheduler pipeline mode", () => {
	it("maxWritesInFlight>1ではparse callbackを待たずに複数writeを発行する", () => {
		const callbacks: Array<() => void> = [];
		const write = vi.fn((_data: string, parsed: () => void) => {
			callbacks.push(parsed);
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation: new ManualContinuation(),
			clock: () => 0,
			maxWritesInFlight: 4,
		});

		scheduler.enqueue("a".repeat(16 * 1024 * 3));

		expect(write).toHaveBeenCalledTimes(3);
		expect(scheduler.metrics().currentQueuedCodeUnits).toBe(16 * 1024 * 3);
		for (const parsed of callbacks.splice(0)) parsed();
		expect(scheduler.metrics().currentQueuedCodeUnits).toBe(0);
	});

	it("maxWritesInFlight上限に達したら残りはparse完了後に発行する", () => {
		const callbacks: Array<() => void> = [];
		const write = vi.fn((_data: string, parsed: () => void) => {
			callbacks.push(parsed);
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation: new ManualContinuation(),
			clock: () => 0,
			maxWritesInFlight: 2,
		});

		scheduler.enqueue("a".repeat(16 * 1024 * 3));

		expect(write).toHaveBeenCalledTimes(2);
		callbacks.shift()?.();
		expect(write).toHaveBeenCalledTimes(3);
	});

	it("setMaxWritesInFlightで直列から切替できる", () => {
		const callbacks: Array<() => void> = [];
		const write = vi.fn((_data: string, parsed: () => void) => {
			callbacks.push(parsed);
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation: new ManualContinuation(),
			clock: () => 0,
		});
		scheduler.setMaxWritesInFlight(8);

		scheduler.enqueue("a".repeat(16 * 1024 * 2));

		expect(write).toHaveBeenCalledTimes(2);
	});

	it("既定はこれまで通り1 write in-flightの直列を維持する", () => {
		const callbacks: Array<() => void> = [];
		const write = vi.fn((_data: string, parsed: () => void) => {
			callbacks.push(parsed);
		});
		const scheduler = new TerminalOutputScheduler({
			write,
			continuation: new ManualContinuation(),
			clock: () => 0,
		});

		scheduler.enqueue("a".repeat(16 * 1024 * 2));

		expect(write).toHaveBeenCalledTimes(1);
		callbacks.shift()?.();
		expect(write).toHaveBeenCalledTimes(2);
	});
});
