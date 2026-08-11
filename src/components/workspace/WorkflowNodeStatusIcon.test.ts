import { describe, expect, it } from "vitest";
import { agentSessionIconPresentation } from "./WorkflowNodeStatusIcon";

describe("agentSessionIconPresentation", () => {
	it("open＋runningはworkflow node runningと同じblue＋pulseになる", () => {
		expect(
			agentSessionIconPresentation({
				lifecycle: "open",
				activity: "running",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-blue-600 dark:text-blue-300",
			pulse: true,
			statusLabel: "running",
		});
	});

	it("open＋idleはニュートラル色でpulseしない", () => {
		expect(
			agentSessionIconPresentation({
				lifecycle: "open",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-foreground",
			pulse: false,
			statusLabel: "open",
		});
	});

	it("paused＋異常終了はdestructiveになる", () => {
		expect(
			agentSessionIconPresentation({
				lifecycle: "paused",
				activity: "idle",
				lastExitAbnormal: true,
			}),
		).toEqual({
			className: "text-destructive",
			pulse: false,
			statusLabel: "paused (exited abnormally)",
		});
	});

	it("paused（正常）は非活性のdimになる", () => {
		expect(
			agentSessionIconPresentation({
				lifecycle: "paused",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-muted-foreground",
			pulse: false,
			statusLabel: "paused",
		});
	});

	it("archivedは非活性のdimになる", () => {
		expect(
			agentSessionIconPresentation({
				lifecycle: "archived",
				activity: "idle",
				lastExitAbnormal: false,
			}),
		).toEqual({
			className: "text-muted-foreground",
			pulse: false,
			statusLabel: "archived",
		});
	});
});
