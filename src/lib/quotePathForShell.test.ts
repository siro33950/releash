import { describe, expect, it } from "vitest";
import { quotePathForShell, quotePathsForShell } from "./quotePathForShell";

describe("quotePathForShell", () => {
	it("should return plain path as-is", () => {
		expect(quotePathForShell("/usr/local/bin/node")).toBe(
			"/usr/local/bin/node",
		);
	});

	it("should quote path with spaces", () => {
		expect(quotePathForShell("/Users/me/My Documents/file.txt")).toBe(
			"'/Users/me/My Documents/file.txt'",
		);
	});

	it("should escape single quotes in path", () => {
		expect(quotePathForShell("/tmp/it's a file")).toBe(
			"'/tmp/it'\\''s a file'",
		);
	});

	it("should quote path with special characters", () => {
		expect(quotePathForShell("/tmp/file(1).txt")).toBe("'/tmp/file(1).txt'");
		expect(quotePathForShell("/tmp/$HOME")).toBe("'/tmp/$HOME'");
		expect(quotePathForShell("/tmp/a&b")).toBe("'/tmp/a&b'");
		expect(quotePathForShell("/tmp/a*")).toBe("'/tmp/a*'");
	});

	it("should not quote simple paths", () => {
		expect(quotePathForShell("/home/user/project/src/main.rs")).toBe(
			"/home/user/project/src/main.rs",
		);
		expect(quotePathForShell("relative/path.txt")).toBe("relative/path.txt");
	});
});

describe("quotePathsForShell", () => {
	it("should join multiple paths with spaces", () => {
		expect(
			quotePathsForShell(["/tmp/a.txt", "/tmp/my file.txt", "/tmp/b.txt"]),
		).toBe("/tmp/a.txt '/tmp/my file.txt' /tmp/b.txt");
	});

	it("should return empty string for empty array", () => {
		expect(quotePathsForShell([])).toBe("");
	});
});
