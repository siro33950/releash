import { describe, expect, it } from "vitest";
import { createCodexSdk } from "./codex-sdk-bridge-entrypoint.mjs";

describe("codex bridge entrypoint", () => {
	it("constructs Codex with an explicit external CLI override", () => {
		const calls = [];
		class FakeCodex {
			constructor(options) {
				calls.push(options);
			}
		}

		createCodexSdk("/opt/bin/codex", FakeCodex);

		expect(calls).toEqual([{ codexPathOverride: "/opt/bin/codex" }]);
	});

	it("defaults to the PATH codex command", () => {
		const calls = [];
		class FakeCodex {
			constructor(options) {
				calls.push(options);
			}
		}

		createCodexSdk("", FakeCodex);

		expect(calls).toEqual([{ codexPathOverride: "codex" }]);
	});
});
