const OUTPUT_CHUNK_CODE_UNITS = 16 * 1024;
const MAX_WRITES_PER_DRAIN = 8;
const MAX_DRAIN_MS = 8;
const LONG_STALL_MS = 100;
const MAX_QUEUED_CODE_UNITS = 2 * 1024 * 1024;

export interface TerminalOutputContinuation {
	post(task: () => void): void;
	dispose(): void;
}

class MessageChannelContinuation implements TerminalOutputContinuation {
	private readonly channel = new MessageChannel();
	private readonly tasks: Array<() => void> = [];
	private posted = false;

	constructor() {
		this.channel.port1.onmessage = () => {
			this.posted = false;
			this.tasks.shift()?.();
			if (this.tasks.length > 0) this.schedule();
		};
	}

	post(task: () => void): void {
		this.tasks.push(task);
		this.schedule();
	}

	dispose(): void {
		this.tasks.length = 0;
		this.channel.port1.close();
		this.channel.port2.close();
	}

	private schedule(): void {
		if (this.posted) return;
		this.posted = true;
		this.channel.port2.postMessage(null);
	}
}

export function createMessageChannelContinuation(): TerminalOutputContinuation {
	return new MessageChannelContinuation();
}

export interface TerminalOutputSchedulerMetrics {
	currentQueuedCodeUnits: number;
	peakQueuedCodeUnits: number;
	writeCount: number;
	longStallsOver100Ms: number;
	droppedBacklogs: number;
	snapshotResyncs: number;
}

interface TerminalOutputSchedulerOptions {
	write: (data: string, parsed: () => void) => void;
	continuation?: TerminalOutputContinuation;
	clock?: () => number;
	onOverflow?: () => void;
	onMetrics?: (metrics: TerminalOutputSchedulerMetrics) => void;
	onParsed?: () => void;
	maxWritesInFlight?: number;
}

interface TerminalOutputSegment {
	data: string;
	offset: number;
	onParsed?: () => void;
}

interface TerminalOutputChunk {
	data: string;
	completedSegments: Array<() => void>;
}

export class TerminalOutputScheduler {
	private readonly write: TerminalOutputSchedulerOptions["write"];
	private readonly continuation: TerminalOutputContinuation;
	private readonly clock: () => number;
	private readonly onOverflow: () => void;
	private readonly onMetrics?: (
		metrics: TerminalOutputSchedulerMetrics,
	) => void;
	private readonly onParsed?: () => void;
	private queue: TerminalOutputSegment[] = [];
	private maxWritesInFlight: number;
	private writesInFlight = 0;
	private inFlightCodeUnits = 0;
	private continuationPending = false;
	private writesInDrain = 0;
	private drainStartedAt: number | null = null;
	private disposed = false;
	private drainWaiters: Array<() => void> = [];
	private currentQueuedCodeUnits = 0;
	private peakQueuedCodeUnits = 0;
	private writeCount = 0;
	private longStallsOver100Ms = 0;
	private droppedBacklogs = 0;
	private snapshotResyncs = 0;
	private awaitingSnapshot = false;

	constructor(options: TerminalOutputSchedulerOptions) {
		this.write = options.write;
		this.continuation =
			options.continuation ?? createMessageChannelContinuation();
		this.clock = options.clock ?? (() => performance.now());
		this.onOverflow = options.onOverflow ?? (() => {});
		this.onMetrics = options.onMetrics;
		this.onParsed = options.onParsed;
		this.maxWritesInFlight = Math.max(1, options.maxWritesInFlight ?? 1);
	}

	setMaxWritesInFlight(count: number): void {
		this.maxWritesInFlight = Math.max(1, count);
	}

	enqueue(data: string, onParsed?: () => void): void {
		if (this.disposed || this.awaitingSnapshot || data.length === 0) return;
		if (this.currentQueuedCodeUnits + data.length > MAX_QUEUED_CODE_UNITS) {
			this.queue = [];
			this.currentQueuedCodeUnits = this.inFlightCodeUnits;
			this.awaitingSnapshot = true;
			this.droppedBacklogs += 1;
			this.emitMetrics();
			this.onOverflow();
			return;
		}
		this.queue.push({ data, offset: 0, onParsed });
		this.currentQueuedCodeUnits += data.length;
		this.peakQueuedCodeUnits = Math.max(
			this.peakQueuedCodeUnits,
			this.currentQueuedCodeUnits,
		);
		this.emitMetrics();
		this.pump();
	}

	resumeAfterSnapshot(): void {
		if (!this.awaitingSnapshot || this.disposed) return;
		this.awaitingSnapshot = false;
		this.snapshotResyncs += 1;
		this.emitMetrics();
	}

	drain(): Promise<void> {
		if (this.currentQueuedCodeUnits === 0) return Promise.resolve();
		return new Promise((resolve) => this.drainWaiters.push(resolve));
	}

	metrics(): TerminalOutputSchedulerMetrics {
		return {
			currentQueuedCodeUnits: this.currentQueuedCodeUnits,
			peakQueuedCodeUnits: this.peakQueuedCodeUnits,
			writeCount: this.writeCount,
			longStallsOver100Ms: this.longStallsOver100Ms,
			droppedBacklogs: this.droppedBacklogs,
			snapshotResyncs: this.snapshotResyncs,
		};
	}

	dispose(): void {
		if (this.disposed) return;
		this.disposed = true;
		this.queue = [];
		this.currentQueuedCodeUnits = 0;
		this.inFlightCodeUnits = 0;
		this.continuation.dispose();
		this.resolveDrainWaiters();
	}

	private pump(): void {
		if (
			this.disposed ||
			this.writesInFlight >= this.maxWritesInFlight ||
			this.continuationPending ||
			this.currentQueuedCodeUnits === 0
		) {
			if (this.currentQueuedCodeUnits === 0) this.finishDrain();
			return;
		}
		if (this.currentQueuedCodeUnits === this.inFlightCodeUnits) return;
		const now = this.clock();
		if (this.drainStartedAt === null) this.drainStartedAt = now;
		if (
			this.writesInDrain >= MAX_WRITES_PER_DRAIN ||
			now - this.drainStartedAt >= MAX_DRAIN_MS
		) {
			this.continuationPending = true;
			this.continuation.post(() => {
				this.continuationPending = false;
				this.writesInDrain = 0;
				this.drainStartedAt = null;
				this.pump();
			});
			return;
		}

		const { data, completedSegments } = this.takeChunk();
		const codeUnits = data.length;
		const startedAt = this.clock();
		this.writesInFlight += 1;
		this.inFlightCodeUnits += codeUnits;
		let completed = false;
		this.write(data, () => {
			if (completed) return;
			completed = true;
			const elapsed = this.clock() - startedAt;
			if (elapsed > LONG_STALL_MS) this.longStallsOver100Ms += 1;
			this.currentQueuedCodeUnits -= codeUnits;
			this.inFlightCodeUnits = Math.max(0, this.inFlightCodeUnits - codeUnits);
			this.writeCount += 1;
			this.writesInDrain += 1;
			this.writesInFlight = Math.max(0, this.writesInFlight - 1);
			this.onParsed?.();
			for (const complete of completedSegments) complete();
			this.emitMetrics();
			this.pump();
		});
		this.pump();
	}

	private takeChunk(): TerminalOutputChunk {
		let remaining = OUTPUT_CHUNK_CODE_UNITS;
		const parts: string[] = [];
		const completedSegments: Array<() => void> = [];
		while (remaining > 0 && this.queue.length > 0) {
			const current = this.queue[0];
			const size = terminalOutputChunkLength(
				current.data,
				current.offset,
				remaining,
			);
			if (size === 0) break;
			parts.push(current.data.slice(current.offset, current.offset + size));
			current.offset += size;
			remaining -= size;
			if (current.offset === current.data.length) {
				this.queue.shift();
				if (current.onParsed) completedSegments.push(current.onParsed);
			}
		}
		return { data: parts.join(""), completedSegments };
	}

	private finishDrain(): void {
		this.writesInDrain = 0;
		this.drainStartedAt = null;
		this.resolveDrainWaiters();
	}

	private resolveDrainWaiters(): void {
		const waiters = this.drainWaiters;
		this.drainWaiters = [];
		for (const resolve of waiters) resolve();
	}

	private emitMetrics(): void {
		this.onMetrics?.(this.metrics());
	}
}

export function terminalOutputChunkLength(
	data: string,
	offset: number,
	maximum: number,
): number {
	const available = data.length - offset;
	const size = Math.min(maximum, available);
	const boundary = offset + size;
	if (
		boundary < data.length &&
		isHighSurrogate(data.charCodeAt(boundary - 1)) &&
		isLowSurrogate(data.charCodeAt(boundary))
	) {
		return size - 1;
	}
	return size;
}

function isHighSurrogate(codeUnit: number): boolean {
	return codeUnit >= 0xd800 && codeUnit <= 0xdbff;
}

function isLowSurrogate(codeUnit: number): boolean {
	return codeUnit >= 0xdc00 && codeUnit <= 0xdfff;
}
