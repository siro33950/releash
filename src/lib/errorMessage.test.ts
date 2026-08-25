import { describe, expect, it } from "vitest";
import { getErrorMessage } from "./errorMessage";

describe("getErrorMessage", () => {
	it("Errorのmessageを返す", () => {
		expect(getErrorMessage(new Error("error message"))).toBe("error message");
	});

	it("messageを持つobjectから文字列を返す", () => {
		expect(
			getErrorMessage({ code: "CODED_ERROR", message: "backend message" }),
		).toBe("backend message");
	});

	it("プレーン文字列をそのまま返す", () => {
		expect(getErrorMessage("plain message")).toBe("plain message");
	});

	it.each([
		[42, "42"],
		[null, "null"],
		[undefined, "undefined"],
		[{ message: 42 }, "[object Object]"],
	])("その他の値%jはStringで変換する", (error, expected) => {
		expect(getErrorMessage(error)).toBe(expected);
	});
});
