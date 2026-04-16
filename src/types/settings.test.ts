import { describe, expect, it } from "vitest";
import {
	type AppSettings,
	buildTerminalCommand,
	DEFAULT_SETTINGS,
} from "./settings";

function makeSettings(overrides: Partial<AppSettings> = {}): AppSettings {
	return { ...DEFAULT_SETTINGS, ...overrides };
}

describe("buildTerminalCommand", () => {
	it("returns empty string for none agent", () => {
		const settings = makeSettings({ agent: "none" });
		expect(buildTerminalCommand(settings)).toBe("");
	});

	it("returns agent command without bypass flag when autoApprove is false", () => {
		const settings = makeSettings({ agent: "claude", agentAutoApprove: false });
		expect(buildTerminalCommand(settings)).toBe("claude");
	});

	it("returns agent command with bypass flag when autoApprove is true", () => {
		const settings = makeSettings({ agent: "claude", agentAutoApprove: true });
		expect(buildTerminalCommand(settings)).toBe(
			"claude --dangerously-skip-permissions",
		);
	});

	it("returns custom terminal command for custom agent", () => {
		const settings = makeSettings({
			agent: "custom",
			terminalStartupCommand: "my-agent --flag",
		});
		expect(buildTerminalCommand(settings)).toBe("my-agent --flag");
	});
});
