import type { TerminalPerformanceFixtureDescriptor } from "./terminalPerformanceReport";

const TARGET_BYTES = 10 * 1024 * 1024;
const AGENT_TUI_FRAME =
	"\u001b[38;5;220m◆ tool\u001b[0m 日本語🙂 wide\r\n" +
	"\u001b[2K\r\u001b[32m✓ completed\u001b[0m\r\n" +
	"\u001b[2A\u001b[12C\u001b[1mredraw\u001b[0m\u001b[2B\r\n" +
	"history-line 日本語🙂\r\n";

export function createAgentTuiFixture(): {
	data: string;
	descriptor: TerminalPerformanceFixtureDescriptor;
} {
	const encoder = new TextEncoder();
	const frameBytes = encoder.encode(AGENT_TUI_FRAME).byteLength;
	const frameCount = Math.ceil(TARGET_BYTES / frameBytes);
	const data = AGENT_TUI_FRAME.repeat(frameCount);
	return {
		data,
		descriptor: {
			kind: "agent-tui",
			byteLength: encoder.encode(data).byteLength,
			containsAnsi: true,
			containsUnicode: true,
			containsWideCharacters: true,
			containsCursorRedraw: true,
		},
	};
}
