import { describe, expect, it, vi } from "vitest";
import {
	StartupInputBuffer,
	TERMINAL_STARTUP_INPUT_BUFFER_LIMIT,
} from "./terminalStartupInputBuffer";

describe("StartupInputBuffer", () => {
	it("limitちょうどまでの入力は破棄せず保持する", () => {
		const onOverflow = vi.fn();
		const buffer = new StartupInputBuffer(onOverflow, 4);

		buffer.push("ab");
		buffer.push("cd");

		expect(onOverflow).not.toHaveBeenCalled();
		expect(buffer.markDone()).toEqual(["ab", "cd"]);
	});

	it("limitを1文字でも超えるchunkは丸ごと破棄しonOverflowへ渡す", () => {
		const onOverflow = vi.fn();
		const buffer = new StartupInputBuffer(onOverflow, 4);

		buffer.push("abcd");
		buffer.push("e");

		expect(onOverflow).toHaveBeenCalledTimes(1);
		expect(onOverflow).toHaveBeenCalledWith("e");
		expect(buffer.markDone()).toEqual(["abcd"]);
	});

	it("既定limitは1024文字で境界も同じ規則に従う", () => {
		const onOverflow = vi.fn();
		const buffer = new StartupInputBuffer(onOverflow);

		buffer.push("a".repeat(TERMINAL_STARTUP_INPUT_BUFFER_LIMIT));
		expect(onOverflow).not.toHaveBeenCalled();

		buffer.push("x");
		expect(onOverflow).toHaveBeenCalledWith("x");
		expect(buffer.markDone()).toEqual([
			"a".repeat(TERMINAL_STARTUP_INPUT_BUFFER_LIMIT),
		]);
	});

	it("markDone後のpushは保持もoverflow通知もしない", () => {
		const onOverflow = vi.fn();
		const buffer = new StartupInputBuffer(onOverflow, 4);

		buffer.push("a");
		expect(buffer.markDone()).toEqual(["a"]);

		buffer.push("b");

		expect(onOverflow).not.toHaveBeenCalled();
		expect(buffer.isDone).toBe(true);
		expect(buffer.markDone()).toEqual([]);
	});

	it("markDoneはpush順のchunkを返す", () => {
		const buffer = new StartupInputBuffer(() => {}, 16);

		buffer.push("1");
		buffer.push("2");
		buffer.push("3");

		expect(buffer.markDone()).toEqual(["1", "2", "3"]);
	});

	it("再markDoneは空配列を返しisDoneを維持する", () => {
		const buffer = new StartupInputBuffer(() => {}, 4);

		buffer.push("a");
		expect(buffer.markDone()).toEqual(["a"]);
		expect(buffer.markDone()).toEqual([]);
		expect(buffer.isDone).toBe(true);
	});
});
