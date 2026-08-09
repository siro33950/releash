export const TERMINAL_STARTUP_INPUT_BUFFER_LIMIT = 1024;

// 起動完了（初回snapshot適用）前の打鍵は破棄せず順序どおり保持する
export class StartupInputBuffer {
	private chunks: string[] = [];
	private bufferedLength = 0;
	private done = false;

	constructor(
		private readonly onOverflow: (dropped: string) => void,
		private readonly limit: number = TERMINAL_STARTUP_INPUT_BUFFER_LIMIT,
	) {}

	get isDone(): boolean {
		return this.done;
	}

	push(data: string): void {
		if (this.done) return;
		if (this.bufferedLength + data.length > this.limit) {
			this.onOverflow(data);
			return;
		}
		this.chunks.push(data);
		this.bufferedLength += data.length;
	}

	markDone(): string[] {
		if (this.done) return [];
		this.done = true;
		const flushed = this.chunks;
		this.chunks = [];
		this.bufferedLength = 0;
		return flushed;
	}
}
