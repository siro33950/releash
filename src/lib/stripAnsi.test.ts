import { describe, expect, it } from "vitest";
import { stripAnsi } from "./stripAnsi";

describe("stripAnsi", () => {
	it("should return plain text unchanged", () => {
		expect(stripAnsi("hello world")).toBe("hello world");
	});

	it("should preserve newlines and tabs", () => {
		expect(stripAnsi("line1\n\tline2\nline3")).toBe("line1\n\tline2\nline3");
	});

	it("should handle empty string", () => {
		expect(stripAnsi("")).toBe("");
	});

	// --- CSI sequences ---

	it("should strip SGR color codes", () => {
		expect(stripAnsi("\u001B[31mred\u001B[0m")).toBe("red");
	});

	it("should strip bold/underline sequences", () => {
		expect(stripAnsi("\u001B[1mbold\u001B[22m")).toBe("bold");
	});

	it("should strip 24-bit color sequences", () => {
		expect(stripAnsi("\u001B[38;2;255;128;0mcolored\u001B[0m")).toBe("colored");
	});

	it("should strip cursor movement sequences", () => {
		expect(stripAnsi("\u001B[2Ahello")).toBe("hello");
		expect(stripAnsi("\u001B[10Bworld")).toBe("world");
		expect(stripAnsi("\u001B[5Cright")).toBe("right");
		expect(stripAnsi("\u001B[3Dleft")).toBe("left");
	});

	it("should strip erase sequences", () => {
		expect(stripAnsi("\u001B[2Jhello")).toBe("hello");
		expect(stripAnsi("\u001B[Khello")).toBe("hello");
	});

	it("should strip DEC private mode sequences", () => {
		expect(stripAnsi("\u001B[?25hhello")).toBe("hello");
		expect(stripAnsi("\u001B[?25lhello")).toBe("hello");
		expect(stripAnsi("\u001B[?1049hhello")).toBe("hello");
	});

	it("should strip scroll sequences", () => {
		expect(stripAnsi("\u001B[2Shello")).toBe("hello");
		expect(stripAnsi("\u001B[3Thello")).toBe("hello");
	});

	it("should strip cursor save/restore CSI sequences", () => {
		expect(stripAnsi("\u001B[shello\u001B[u")).toBe("hello");
	});

	it("should strip multiple sequences in a row", () => {
		expect(stripAnsi("\u001B[1m\u001B[31mhello\u001B[0m")).toBe("hello");
	});

	it("should handle mixed content with CSI sequences", () => {
		const input =
			"\u001B[32m✓\u001B[0m Analyzing \u001B[1msrc/foo.ts\u001B[0m...";
		expect(stripAnsi(input)).toBe("✓ Analyzing src/foo.ts...");
	});

	// --- OSC sequences ---

	it("should strip OSC sequences terminated with BEL", () => {
		expect(stripAnsi("\u001B]0;window title\u0007hello")).toBe("hello");
	});

	it("should strip OSC sequences terminated with ST", () => {
		expect(stripAnsi("\u001B]0;window title\u001B\\hello")).toBe("hello");
	});

	it("should strip OSC hyperlink sequences", () => {
		const input =
			"\u001B]8;;https://example.com\u0007link text\u001B]8;;\u0007";
		expect(stripAnsi(input)).toBe("link text");
	});

	// --- Two-character ESC sequences ---

	it("should strip DECSC/DECRC (save/restore cursor)", () => {
		expect(stripAnsi("\u001B7hello\u001B8")).toBe("hello");
	});

	it("should strip RIS (reset)", () => {
		expect(stripAnsi("\u001Bchello")).toBe("hello");
	});

	it("should strip IND/NEL/RI sequences", () => {
		expect(stripAnsi("\u001BDhello")).toBe("hello");
		expect(stripAnsi("\u001BEhello")).toBe("hello");
		expect(stripAnsi("\u001BMhello")).toBe("hello");
	});

	// --- Carriage return processing ---

	it("should process carriage returns (line overwrite)", () => {
		expect(stripAnsi("loading...\rdone!")).toBe("done!ng...");
	});

	it("should handle full line overwrite with CR", () => {
		expect(stripAnsi("AAAA\rBBBB")).toBe("BBBB");
	});

	it("should handle partial overwrite with CR", () => {
		expect(stripAnsi("ABCDEF\rXY")).toBe("XYCDEF");
	});

	it("should handle multiple CRs", () => {
		// "first"(5) → "second"(6) overwrites all → "third"(5) overwrites first 5 of "second", leaving "d"
		expect(stripAnsi("first\rsecond\rthird")).toBe("thirdd");
	});

	it("should handle CR with newlines", () => {
		expect(stripAnsi("progress 10%\rprogress 50%\nprogress done")).toBe(
			"progress 50%\nprogress done",
		);
	});

	it("should handle spinner-style output", () => {
		const input = "⠋ Working...\r⠙ Working...\r⠹ Working...\r✓ Done!     ";
		expect(stripAnsi(input)).toBe("✓ Done!     ");
	});

	// --- Control character stripping ---

	it("should strip NUL characters", () => {
		expect(stripAnsi("hello\x00world")).toBe("helloworld");
	});

	it("should strip backspace characters", () => {
		expect(stripAnsi("hello\x08world")).toBe("helloworld");
	});

	it("should strip BEL characters", () => {
		expect(stripAnsi("hello\x07world")).toBe("helloworld");
	});

	it("should strip DEL characters", () => {
		expect(stripAnsi("hello\x7fworld")).toBe("helloworld");
	});

	it("should not strip tab characters", () => {
		expect(stripAnsi("hello\tworld")).toBe("hello\tworld");
	});

	it("should not strip newline characters", () => {
		expect(stripAnsi("hello\nworld")).toBe("hello\nworld");
	});

	// --- Complex real-world scenarios ---

	it("should handle Claude Code style output", () => {
		const input = [
			"\u001B[1m\u001B[34m⏺\u001B[0m Analyzing code...",
			"\u001B[32m✓\u001B[0m Found 3 issues",
			"",
			"\u001B[33m⚠\u001B[0m Warning in \u001B[4msrc/app.ts:42\u001B[24m",
		].join("\n");
		expect(stripAnsi(input)).toBe(
			[
				"⏺ Analyzing code...",
				"✓ Found 3 issues",
				"",
				"⚠ Warning in src/app.ts:42",
			].join("\n"),
		);
	});

	it("should handle combined ANSI + CR output", () => {
		const input =
			"\u001B[36m⠋\u001B[0m Loading...\r\u001B[36m⠙\u001B[0m Loading...\r\u001B[32m✓\u001B[0m Done!       ";
		expect(stripAnsi(input)).toBe("✓ Done!       ");
	});

	// --- C1 control codes ---

	it("should strip C1 CSI sequences (0x9B prefix)", () => {
		expect(stripAnsi("\u009B31mred\u009B0m")).toBe("red");
	});
});
