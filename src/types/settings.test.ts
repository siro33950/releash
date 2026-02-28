import { describe, expect, it } from "vitest";
import {
	type AppSettings,
	buildReviewCommand,
	DEFAULT_SETTINGS,
} from "./settings";

function makeSettings(overrides: Partial<AppSettings> = {}): AppSettings {
	return { ...DEFAULT_SETTINGS, ...overrides };
}

describe("buildReviewCommand", () => {
	const prompt = "Review this code for bugs";
	const skill = "code-review";

	it("returns claude command with default model", () => {
		const settings = makeSettings({ reviewAgent: "claude", reviewModel: "" });
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe(
			'echo "/code-review" | claude -p --verbose --output-format stream-json --permission-mode bypassPermissions --allowedTools "Read,Bash,Glob,Grep,mcp__releash__worktrees_list,mcp__releash__post_review_comment,mcp__releash__get_review_comments,mcp__releash__resolve_comment"',
		);
	});

	it("returns claude command with specified model", () => {
		const settings = makeSettings({
			reviewAgent: "claude",
			reviewModel: "claude-sonnet-4-5-20250929",
		});
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe(
			'echo "/code-review" | claude -p --verbose --output-format stream-json --permission-mode bypassPermissions --allowedTools "Read,Bash,Glob,Grep,mcp__releash__worktrees_list,mcp__releash__post_review_comment,mcp__releash__get_review_comments,mcp__releash__resolve_comment" --model claude-sonnet-4-5-20250929',
		);
	});

	it("returns codex command", () => {
		const settings = makeSettings({ reviewAgent: "codex", reviewModel: "" });
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe(
			'codex exec --sandbox read-only --ask-for-approval never --json "code-review"',
		);
	});

	it("returns gemini command with model", () => {
		const settings = makeSettings({
			reviewAgent: "gemini",
			reviewModel: "gemini-2.5-pro",
		});
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe(
			'gemini -p --sandbox --output-format json --model gemini-2.5-pro "/code-review"',
		);
	});

	it("returns aider command with prompt", () => {
		const settings = makeSettings({ reviewAgent: "aider", reviewModel: "" });
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe(
			'aider --message --yes-always "Review this code for bugs"',
		);
	});

	it("returns null for none agent", () => {
		const settings = makeSettings({ reviewAgent: "none" });
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBeNull();
	});

	it("returns cursor command with skill", () => {
		const settings = makeSettings({
			reviewAgent: "cursor",
			reviewModel: "",
		});
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe('cursor-agent -p --output-format json "/code-review"');
	});

	it("returns custom command with prompt substitution", () => {
		const settings = makeSettings({
			reviewAgent: "custom",
			customReviewCommand: 'my-tool --review "{prompt}"',
		});
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBe('my-tool --review "Review this code for bugs"');
	});

	it("returns null for custom agent with empty command", () => {
		const settings = makeSettings({
			reviewAgent: "custom",
			customReviewCommand: "",
		});
		const result = buildReviewCommand(settings, prompt, skill);
		expect(result).toBeNull();
	});

	it("escapes double quotes in prompt for aider", () => {
		const settings = makeSettings({ reviewAgent: "aider", reviewModel: "" });
		const result = buildReviewCommand(settings, 'Review "this" code', skill);
		expect(result).toBe(
			'aider --message --yes-always "Review \\"this\\" code"',
		);
	});

	it("uses custom skill name", () => {
		const settings = makeSettings({ reviewAgent: "claude", reviewModel: "" });
		const result = buildReviewCommand(settings, prompt, "security-check");
		expect(result).toBe(
			'echo "/security-check" | claude -p --verbose --output-format stream-json --permission-mode bypassPermissions --allowedTools "Read,Bash,Glob,Grep,mcp__releash__worktrees_list,mcp__releash__post_review_comment,mcp__releash__get_review_comments,mcp__releash__resolve_comment"',
		);
	});
});
